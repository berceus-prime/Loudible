use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::execution_context::{ExportExecutionContext, ExportProgressEvent};
use super::file_finalizer::ExportFileFinalizer;
use super::license_provider::LicenseProvider;
use crate::export_file_naming::base_name;
use crate::{
    Audiobook, ContentLicense, DownloadManifest, DownloadRecovery, ExporterError, FileDownloader,
    JobKind, LocalDownloads, PartialDownload, Phase,
};

pub struct DownloadResult {
    pub input: PathBuf,
    pub fresh_license: Option<ContentLicense>,
}

/// Downloads encrypted audio and persists application-owned recovery and metadata.
#[derive(Clone)]
pub struct DownloadWorkflow {
    pub licenses: LicenseProvider,
    pub downloader: Arc<dyn FileDownloader>,
    pub downloads: LocalDownloads,
    pub finalizer: ExportFileFinalizer,
}

impl DownloadWorkflow {
    pub async fn run(
        &self,
        book: &Audiobook,
        kind: JobKind,
        context: &ExportExecutionContext,
    ) -> Result<DownloadResult, ExporterError> {
        if let Some(existing) = self.downloads.scan_downloads().remove(&book.asin) {
            if kind == JobKind::Download {
                context.phase(Phase::Finalizing);
            }
            context.check_cancellation()?;
            return Ok(DownloadResult {
                input: existing,
                fresh_license: None,
            });
        }
        context.phase(Phase::Licensing);
        let (license, customer_id) = self.fetch_license(book, context).await?;
        context.check_cancellation()?;
        let input = self
            .download(book, &license, &customer_id, kind, context)
            .await?;
        Ok(DownloadResult {
            input,
            fresh_license: Some(license),
        })
    }

    /// Dropping the fetch abandons its request, as a cancelled URLSession call did.
    pub async fn fetch_license(
        &self,
        book: &Audiobook,
        context: &ExportExecutionContext,
    ) -> Result<(ContentLicense, String), ExporterError> {
        tokio::select! {
            _ = context.cancel.cancelled() => Err(ExporterError::Cancelled),
            result = self.licenses.fetch(book) => result,
        }
    }

    /// Downloads into a hidden staging file, writes the manifest, then moves the file
    /// to `Title [ASIN].<ext>`. The license is always fresh, since its download URL expires.
    /// Failure or suspension keeps recovery state; explicit cancellation discards it.
    async fn download(
        &self,
        book: &Audiobook,
        license: &ContentLicense,
        customer_id: &str,
        kind: JobKind,
        context: &ExportExecutionContext,
    ) -> Result<PathBuf, ExporterError> {
        let cover =
            self.downloads
                .manifest_folder
                .join(format!(".{}.{}.cover", book.asin, Uuid::new_v4()));
        let result = self
            .transfer(book, license, customer_id, kind, context, &cover)
            .await;
        let _ = fs::remove_file(&cover);
        if result.is_err() && context.cancel.is_cancelled() && !(context.keeps_recovery)() {
            self.downloads.remove_partial_download(&book.asin);
        }
        result
    }

    async fn transfer(
        &self,
        book: &Audiobook,
        license: &ContentLicense,
        customer_id: &str,
        kind: JobKind,
        context: &ExportExecutionContext,
        cover: &Path,
    ) -> Result<PathBuf, ExporterError> {
        let resume_data = self.prepare_recovery(book, license, customer_id)?;
        let partial = self.downloads.partial_file_path(&book.asin);
        context.phase(Phase::Downloading);
        let invocation = Uuid::new_v4();
        (context.report)(ExportProgressEvent::BeginDownload(invocation));

        let recovery = {
            let downloads = self.downloads.clone();
            let book = book.clone();
            let customer_id = customer_id.to_string();
            let decryption = license.decryption.clone();
            Arc::new(move |recovery: DownloadRecovery| {
                // A failed recovery write leaves the initial record to restart from zero.
                let _ = downloads.write_partial_download(&PartialDownload::new(
                    book.clone(),
                    customer_id.clone(),
                    decryption.file_extension(),
                    recovery,
                    decryption.clone(),
                    Utc::now(),
                ));
            })
        };
        let progress = {
            let report = context.report.clone();
            Arc::new(move |value| report(ExportProgressEvent::Download(invocation, value)))
        };
        let result = self
            .downloader
            .download(
                &license.download_url,
                &partial,
                resume_data,
                recovery,
                progress,
                &context.cancel,
            )
            .await;
        (context.report)(ExportProgressEvent::EndDownload(invocation));
        result?;
        context.check_cancellation()?;

        if kind == JobKind::Download {
            context.phase(Phase::Finalizing);
        }
        // The cover is a nicety for books that later leave the library; failing to get it is fine.
        if let Some(cover_url) = &book.cover_image_url {
            let _ = self
                .downloader
                .download(
                    cover_url,
                    cover,
                    None,
                    Arc::new(|_| {}),
                    Arc::new(|_| {}),
                    &context.cancel,
                )
                .await;
        }
        context.check_cancellation()?;
        self.finalize(book, license, cover)
    }

    fn finalize(
        &self,
        book: &Audiobook,
        license: &ContentLicense,
        cover: &Path,
    ) -> Result<PathBuf, ExporterError> {
        let extension = license.decryption.file_extension();
        let base = base_name(book, extension);
        let partial = self.downloads.partial_file_path(&book.asin);
        let manifest = DownloadManifest::new(
            book.clone(),
            license.decryption.clone(),
            license.chapters.clone(),
            Utc::now(),
        );
        self.downloads
            .write_manifest(&manifest, cover.exists().then_some(cover))?;
        let path = self
            .finalizer
            .move_into_place(&partial, &self.downloads.folder, &base, extension)
            .inspect_err(|_| self.downloads.remove_manifest(&book.asin))?;
        self.downloads.remove_partial_download(&book.asin);
        Ok(path)
    }

    fn prepare_recovery(
        &self,
        book: &Audiobook,
        license: &ContentLicense,
        customer_id: &str,
    ) -> Result<Option<Vec<u8>>, ExporterError> {
        let extension = license.decryption.file_extension();
        let usable = self
            .downloads
            .partial_download(&book.asin)
            .filter(|record| {
                record.customer_id == customer_id
                    && record.book.asin == book.asin
                    && record.file_extension == extension
                    && record.decryption == license.decryption
            });
        if usable.is_none() {
            self.downloads.remove_partial_download(&book.asin);
        }
        self.downloads
            .write_partial_download(&PartialDownload::new(
                book.clone(),
                customer_id,
                extension,
                DownloadRecovery::default(),
                license.decryption.clone(),
                Utc::now(),
            ))?;
        Ok(usable.and_then(|record| record.recovery.resume_data))
    }
}
