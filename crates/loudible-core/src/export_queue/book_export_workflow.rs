use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use super::download_workflow::DownloadWorkflow;
use super::execution_context::{ExportExecutionContext, ExportProgressEvent};
use super::file_finalizer::ExportFileFinalizer;
use crate::export_file_naming::{FILE_EXTENSION, base_name, partial_name};
use crate::{
    AudioConverter, Audiobook, BookMetadata, ConversionRequest, ExporterError, JobKind, Phase,
};

pub const REDOWNLOAD_HINT: &str =
    "If this keeps failing, delete the download and download it again.";

/// Converts a kept or newly downloaded book to an M4B.
#[derive(Clone)]
pub struct BookExportWorkflow {
    pub download: DownloadWorkflow,
    pub converter: Arc<dyn AudioConverter>,
    pub finalizer: ExportFileFinalizer,
}

impl BookExportWorkflow {
    pub async fn run(
        &self,
        book: &Audiobook,
        destination: &Path,
        context: &ExportExecutionContext,
    ) -> Result<PathBuf, ExporterError> {
        let downloaded = self.download.run(book, JobKind::Export, context).await?;
        let is_kept_download = downloaded.fresh_license.is_none();

        // Keys: from the license just used, else the manifest, else a fresh license.
        let (decryption, chapters) = if let Some(license) = downloaded.fresh_license {
            (license.decryption, license.chapters)
        } else if let Some(manifest) = self.download.downloads.manifest(&book.asin) {
            (manifest.decryption, manifest.chapters)
        } else {
            context.phase(Phase::Licensing);
            let fresh = self.download.fetch_license(book, context).await?;
            (fresh.decryption, fresh.chapters)
        };
        context.check_cancellation()?;

        let base = base_name(book, FILE_EXTENSION);
        let partial = destination.join(partial_name(&base, &Uuid::new_v4().to_string()));
        let request = ConversionRequest {
            input: downloaded.input,
            output: partial.clone(),
            decryption,
            chapters,
            metadata: BookMetadata::from(book),
        };
        let result = self
            .convert_and_place(request, destination, &base, is_kept_download, context)
            .await;
        let _ = fs::remove_file(&partial);
        result
    }

    async fn convert_and_place(
        &self,
        request: ConversionRequest,
        destination: &Path,
        base: &str,
        is_kept_download: bool,
        context: &ExportExecutionContext,
    ) -> Result<PathBuf, ExporterError> {
        context.phase(Phase::Converting);
        let partial = request.output.clone();
        let report = context.report.clone();
        self.converter
            .convert(
                request,
                Arc::new(move |value| report(ExportProgressEvent::Conversion(value))),
                &context.cancel,
            )
            .await
            .map_err(|error| match error {
                ExporterError::Conversion(detail) if is_kept_download => {
                    ExporterError::Conversion(format!("{detail} {REDOWNLOAD_HINT}"))
                }
                other => other,
            })?;
        context.check_cancellation()?;
        if !partial.exists() {
            return Err(ExporterError::Conversion(
                "The converter did not produce an output file.".to_string(),
            ));
        }

        context.phase(Phase::Finalizing);
        context.check_cancellation()?;
        self.finalizer
            .move_into_place(&partial, destination, base, FILE_EXTENSION)
    }
}
