use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use loudible_core::{
    DownloadProgress, DownloadProgressSink, DownloadRecovery, ExporterError, FileDownloader,
    RecoverySink, TransferProgress,
};
use serde::{Deserialize, Serialize};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::http_transport::{client_builder, describe};

const USER_AGENT: &str =
    "com.audible.playersdk.player/3.96.1 (Linux;Android 14) AndroidXMedia3/1.3.0";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Where an interrupted transfer stopped. The bytes stay in the destination file, so a token is
/// only meaningful with that file, and any failure to use one means starting over.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResumeToken {
    version: u32,
    url: Url,
    etag: Option<String>,
    last_modified: Option<String>,
    received: u64,
    total: Option<u64>,
}

impl ResumeToken {
    const CURRENT_VERSION: u32 = 1;

    fn validator(&self) -> Option<&str> {
        self.etag.as_deref().or(self.last_modified.as_deref())
    }
}

enum Failure {
    Status(u16),
    Network(String),
    Destination(String),
    Cancelled,
}

struct Outcome {
    result: Result<(), Failure>,
    recovery: DownloadRecovery,
}

/// Downloads into `destination` itself, resuming with `Range` and `If-Range`. An unusable token
/// gets one fresh attempt at the supplied URL.
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
}

fn discard_unless_resumable(destination: &Path, recovery: &DownloadRecovery) {
    if recovery.resume_data.is_none() {
        let _ = std::fs::remove_file(destination);
    }
}

#[async_trait]
impl FileDownloader for ReqwestDownloader {
    async fn download(
        &self,
        url: &Url,
        destination: &Path,
        resume_data: Option<Vec<u8>>,
        recovery: RecoverySink,
        progress: DownloadProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        if cancel.is_cancelled() {
            return Err(ExporterError::Cancelled);
        }
        // Clear consumed recovery before starting: a crash must not reuse an outdated token.
        recovery(DownloadRecovery::default());
        let report = |attempt: i64| {
            let progress = progress.clone();
            move |transferred| {
                progress(DownloadProgress {
                    attempt,
                    transferred,
                });
            }
        };
        let is_resuming = resume_data.is_some();
        report(0)(TransferProgress::new(0, 0));
        let first = Attempt {
            client: &self.client,
            url,
            destination,
            token: resume_data,
            progress: report(0),
            cancel,
        }
        .run()
        .await;
        if cancel.is_cancelled() {
            discard_unless_resumable(destination, &first.recovery);
            recovery(first.recovery);
            return Err(ExporterError::Cancelled);
        }
        let can_retry = matches!(first.result, Err(Failure::Status(_) | Failure::Network(_)));
        let outcome = if is_resuming && can_retry {
            let _ = std::fs::remove_file(destination);
            recovery(DownloadRecovery::default());
            report(1)(TransferProgress::new(0, 0));
            Attempt {
                client: &self.client,
                url,
                destination,
                token: None,
                progress: report(1),
                cancel,
            }
            .run()
            .await
        } else {
            first
        };
        recovery(outcome.recovery.clone());
        if cancel.is_cancelled() {
            discard_unless_resumable(destination, &outcome.recovery);
            return Err(ExporterError::Cancelled);
        }
        let Err(failure) = outcome.result else {
            return Ok(());
        };
        discard_unless_resumable(destination, &outcome.recovery);
        Err(match failure {
            Failure::Destination(detail) => {
                ExporterError::FileSystem(format!("Could not save download: {detail}"))
            }
            Failure::Status(status) => ExporterError::Download(format!("HTTP {status}")),
            Failure::Network(detail) => ExporterError::Download(detail),
            Failure::Cancelled => ExporterError::Cancelled,
        })
    }
}

struct Attempt<'a, P> {
    client: &'a reqwest::Client,
    url: &'a Url,
    destination: &'a Path,
    token: Option<Vec<u8>>,
    progress: P,
    cancel: &'a CancellationToken,
}

/// What the transfer has established so far, from which recovery is built when it stops.
struct Transfer {
    url: Url,
    etag: Option<String>,
    last_modified: Option<String>,
    accepts_ranges: bool,
    received: u64,
    total: Option<u64>,
    last_progress: Option<TransferProgress>,
}

impl Transfer {
    fn recovery(&self, keep_token: bool) -> DownloadRecovery {
        let token = ResumeToken {
            version: ResumeToken::CURRENT_VERSION,
            url: self.url.clone(),
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
            received: self.received,
            total: self.total,
        };
        let resumable =
            keep_token && self.accepts_ranges && self.received > 0 && token.validator().is_some();
        DownloadRecovery {
            resume_data: resumable.then(|| serde_json::to_vec(&token).expect("a token serializes")),
            progress: self.last_progress,
        }
    }
}

fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    Some(response.headers().get(name)?.to_str().ok()?.to_string())
}

/// `If-Range` requires a strong validator.
fn strong_etag(response: &reqwest::Response) -> Option<String> {
    header(response, reqwest::header::ETAG).filter(|etag| !etag.starts_with("W/"))
}

/// The first byte and total size from `Content-Range: bytes <first>-<last>/<total>`.
fn content_range(response: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = header(response, reqwest::header::CONTENT_RANGE)?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let first = range.split_once('-')?.0.parse().ok()?;
    Some((first, total.parse().ok()))
}

fn network(detail: &str) -> Failure {
    Failure::Network(detail.to_string())
}

fn destination_failure(error: io::Error) -> Failure {
    Failure::Destination(error.to_string())
}

impl<P: Fn(TransferProgress)> Attempt<'_, P> {
    async fn run(mut self) -> Outcome {
        let token = self.token.take();
        let is_resuming = token.is_some();
        let resume = token.and_then(|bytes| self.usable_token(&bytes));
        let mut transfer = Transfer {
            url: resume
                .as_ref()
                .map_or_else(|| self.url.clone(), |token| token.url.clone()),
            etag: resume.as_ref().and_then(|token| token.etag.clone()),
            last_modified: resume
                .as_ref()
                .and_then(|token| token.last_modified.clone()),
            accepts_ranges: true,
            received: resume.as_ref().map_or(0, |token| token.received),
            total: resume.as_ref().and_then(|token| token.total),
            last_progress: None,
        };
        if is_resuming && resume.is_none() {
            return Outcome {
                result: Err(network("The saved download position cannot be used.")),
                recovery: DownloadRecovery::default(),
            };
        }
        let result = self.transfer(&mut transfer, resume.as_ref()).await;
        // A rejected response says nothing about the bytes we hold.
        let keep_token = !matches!(result, Ok(()) | Err(Failure::Status(_)));
        Outcome {
            recovery: transfer.recovery(keep_token),
            result,
        }
    }

    fn usable_token(&self, bytes: &[u8]) -> Option<ResumeToken> {
        let token: ResumeToken = serde_json::from_slice(bytes).ok()?;
        let size = std::fs::metadata(self.destination).ok()?.len();
        (token.version == ResumeToken::CURRENT_VERSION
            && token.received > 0
            && token.received <= size
            && token.validator().is_some())
        .then_some(token)
    }

    async fn transfer(
        &self,
        transfer: &mut Transfer,
        resume: Option<&ResumeToken>,
    ) -> Result<(), Failure> {
        let mut request = self.client.get(transfer.url.clone());
        if let Some(token) = resume {
            request = request
                .header(reqwest::header::RANGE, format!("bytes={}-", token.received))
                .header(
                    reqwest::header::IF_RANGE,
                    token.validator().unwrap_or_default(),
                );
        }
        let mut response = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(Failure::Cancelled),
            response = request.send() => {
                response.map_err(|error| Failure::Network(describe(error)))?
            }
        };

        let status = response.status().as_u16();
        let is_partial = status == 206;
        if !(matches!(status, 200 | 201 | 203) || is_partial && resume.is_some()) {
            return Err(Failure::Status(status));
        }
        let mut file = if is_partial {
            self.continue_file(transfer, &response).await?
        } else {
            transfer.received = 0;
            transfer.total = response.content_length();
            File::create(self.destination)
                .await
                .map_err(destination_failure)?
        };
        if let Some(etag) = strong_etag(&response) {
            transfer.etag = Some(etag);
        } else if !is_partial {
            transfer.etag = None;
        }
        if let Some(modified) = header(&response, reqwest::header::LAST_MODIFIED) {
            transfer.last_modified = Some(modified);
        } else if !is_partial {
            transfer.last_modified = None;
        }
        transfer.accepts_ranges = header(&response, reqwest::header::ACCEPT_RANGES)
            .is_none_or(|ranges| !ranges.eq_ignore_ascii_case("none"));

        let result = self.receive(transfer, &mut response, &mut file).await;
        // Everything counted in `received` must be on disk before a token claims it.
        let flushed = file.flush().await.map_err(destination_failure);
        result.and(flushed)?;
        drop(file);

        if transfer
            .total
            .is_some_and(|total| transfer.received < total)
        {
            return Err(network(
                "The connection closed before the download finished.",
            ));
        }
        let completed = TransferProgress::new(transfer.received as i64, transfer.received as i64);
        transfer.last_progress = Some(completed);
        (self.progress)(completed);
        Ok(())
    }

    /// Opens the destination at the position the partial response starts from.
    async fn continue_file(
        &self,
        transfer: &mut Transfer,
        response: &reqwest::Response,
    ) -> Result<File, Failure> {
        let (first, total) = content_range(response)
            .filter(|(first, _)| *first == transfer.received)
            .ok_or_else(|| network("The server resumed from the wrong position."))?;
        if total.is_some() {
            transfer.total = total;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .open(self.destination)
            .await
            .map_err(destination_failure)?;
        // Bytes past the saved position were never confirmed written.
        file.set_len(first).await.map_err(destination_failure)?;
        file.seek(SeekFrom::Start(first))
            .await
            .map_err(destination_failure)?;
        Ok(file)
    }

    async fn receive(
        &self,
        transfer: &mut Transfer,
        response: &mut reqwest::Response,
        file: &mut File,
    ) -> Result<(), Failure> {
        let mut last_report: Option<Instant> = None;
        loop {
            // Only the network wait is raced: a write that has started must finish being counted.
            let chunk = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Err(Failure::Cancelled),
                chunk = response.chunk() => {
                    chunk.map_err(|error| Failure::Network(describe(error)))?
                }
            };
            let Some(chunk) = chunk else {
                return Ok(());
            };
            file.write_all(&chunk).await.map_err(destination_failure)?;
            transfer.received += chunk.len() as u64;
            let expected = transfer.total.unwrap_or(0) as i64;
            let value = TransferProgress::new(transfer.received as i64, expected);
            transfer.last_progress = Some(value);
            if expected > 0 && last_report.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL) {
                last_report = Some(Instant::now());
                (self.progress)(value);
            }
        }
    }
}
