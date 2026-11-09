use std::path::Path;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use loudible_core::{DownloadProgressSink, ExporterError, FileDownloader, TransferProgress};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::http_transport::{client_builder, describe};

const USER_AGENT: &str =
    "com.audible.playersdk.player/3.96.1 (Linux;Android 14) AndroidXMedia3/1.3.0";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Downloads each URL from byte zero into a new destination file.
pub struct ReqwestDownloader {
    client: reqwest::Client,
}

impl ReqwestDownloader {
    pub fn new() -> Result<Self, ExporterError> {
        let client = client_builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| ExporterError::Unexpected(describe(error)))?;
        Ok(Self { client })
    }

    async fn transfer(
        &self,
        url: &Url,
        destination: &Path,
        progress: DownloadProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        if cancel.is_cancelled() {
            return Err(ExporterError::Cancelled);
        }
        progress(TransferProgress::new(0, 0));
        let mut response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ExporterError::Cancelled),
            response = self.client.get(url.clone()).send() => {
                response.map_err(|error| ExporterError::Download(describe(error)))?
            }
        };
        let status = response.status().as_u16();
        if !matches!(status, 200 | 201 | 203) {
            return Err(ExporterError::Download(format!("HTTP {status}")));
        }
        let expected = response.content_length();
        let mut file = File::create(destination).await.map_err(|error| {
            ExporterError::FileSystem(format!("Could not save download: {error}"))
        })?;
        let mut received = 0_u64;
        let mut last_report: Option<Instant> = None;
        loop {
            let chunk = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ExporterError::Cancelled),
                chunk = response.chunk() => {
                    chunk.map_err(|error| ExporterError::Download(describe(error)))?
                }
            };
            let Some(chunk) = chunk else { break };
            file.write_all(&chunk).await.map_err(|error| {
                ExporterError::FileSystem(format!("Could not save download: {error}"))
            })?;
            received += chunk.len() as u64;
            if expected.is_some_and(|size| size > 0)
                && last_report.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL)
            {
                last_report = Some(Instant::now());
                progress(TransferProgress::new(
                    received.min(i64::MAX as u64) as i64,
                    expected.unwrap_or(0).min(i64::MAX as u64) as i64,
                ));
            }
        }
        file.flush().await.map_err(|error| {
            ExporterError::FileSystem(format!("Could not save download: {error}"))
        })?;
        drop(file);
        if expected.is_some_and(|size| received < size) {
            return Err(ExporterError::Download(
                "The connection closed before the download finished.".to_string(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(ExporterError::Cancelled);
        }
        let completed = received.min(i64::MAX as u64) as i64;
        progress(TransferProgress::new(completed, completed));
        Ok(())
    }
}

#[async_trait]
impl FileDownloader for ReqwestDownloader {
    async fn download(
        &self,
        url: &Url,
        destination: &Path,
        progress: DownloadProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        let _ = std::fs::remove_file(destination);
        let result = self.transfer(url, destination, progress, cancel).await;
        if result.is_err() {
            let _ = std::fs::remove_file(destination);
        }
        result
    }
}
