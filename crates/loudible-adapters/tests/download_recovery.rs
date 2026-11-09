//! Ported from `DownloadRecoveryTests.swift`, plus the partial-file handling Rust owns itself.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;

use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::*;
use tokio_util::sync::CancellationToken;
use url::Url;

const USER_AGENT: &str =
    "com.audible.playersdk.player/3.96.1 (Linux;Android 14) AndroidXMedia3/1.3.0";

fn payload() -> Vec<u8> {
    (0..500_000u32).map(|i| (i % 251) as u8).collect()
}

struct ServerState {
    expired: bool,
    stalls: bool,
    etag: Option<String>,
    accept_ranges: String,
    requests: Vec<HashMap<String, String>>,
    streams: Vec<TcpStream>,
}

/// Loopback server for recovery tests: stall, serve a range, or expire the original URL.
struct LocalHttpServer {
    state: Arc<Locked<ServerState>>,
    url: Url,
}

impl LocalHttpServer {
    fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "http://127.0.0.1:{}/content.aaxc",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let state = Arc::new(Locked::new(ServerState {
            expired: false,
            stalls: true,
            etag: Some("\"synthetic-v1\"".into()),
            accept_ranges: "bytes".into(),
            requests: Vec::new(),
            streams: Vec::new(),
        }));
        let shared = state.clone();
        let body = Arc::new(body);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (state, body) = (shared.clone(), body.clone());
                std::thread::spawn(move || respond(stream, &state, &body));
            }
        });
        Self { state, url }
    }

    /// Releases stalled transfers on future connections, or makes the original URL expire.
    fn finish_transfers(&self, expired: bool) {
        self.state.with(|state| {
            state.stalls = false;
            state.expired = expired;
        });
    }

    fn requests(&self) -> Vec<HashMap<String, String>> {
        self.state.with(|state| state.requests.clone())
    }

    /// Ends every connection abruptly, as a network failure would.
    fn drop_connections(&self) {
        self.state.with(|state| {
            for stream in state.streams.drain(..) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
    }
}

impl Drop for LocalHttpServer {
    fn drop(&mut self) {
        self.drop_connections();
    }
}

fn respond(mut stream: TcpStream, state: &Locked<ServerState>, body: &[u8]) {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(count) if count > 0 => buffer.extend_from_slice(&chunk[..count]),
            _ => return,
        }
    }
    let text = String::from_utf8_lossy(&buffer).into_owned();
    let headers: HashMap<String, String> = text
        .split("\r\n")
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_string()))
        .collect();
    let (expired, stalls, etag, accept_ranges) = state.with(|state| {
        state.requests.push(headers.clone());
        state.streams.push(stream.try_clone().unwrap());
        (
            state.expired,
            state.stalls,
            state.etag.clone(),
            state.accept_ranges.clone(),
        )
    });
    let validator_matches = headers
        .get("if-range")
        .is_none_or(|v| Some(v) == etag.as_ref());
    let offset: usize = headers
        .get("range")
        .filter(|_| validator_matches)
        .and_then(|range| {
            range
                .strip_prefix("bytes=")?
                .strip_suffix('-')?
                .parse()
                .ok()
        })
        .unwrap_or(0);
    let start = offset.min(body.len());
    let status = if expired {
        403
    } else if start > 0 {
        206
    } else {
        200
    };
    let remaining: &[u8] = if expired {
        b"expired URL"
    } else {
        &body[start..]
    };
    let mut lines = vec![
        format!("HTTP/1.1 {status} Test"),
        format!("Content-Length: {}", remaining.len()),
        "Content-Type: application/octet-stream".to_string(),
        "Connection: close".to_string(),
        format!("Accept-Ranges: {accept_ranges}"),
    ];
    if let Some(etag) = etag {
        lines.push(format!("ETag: {etag}"));
    }
    if status == 206 {
        lines.push(format!(
            "Content-Range: bytes {start}-{}/{}",
            body.len() - 1,
            body.len()
        ));
    }
    let head = lines.join("\r\n") + "\r\n\r\n";
    let _ = stream.write_all(head.as_bytes());
    if stalls && !expired {
        // Some bytes to save, then wait for the client to give up.
        let _ = stream.write_all(&remaining[..remaining.len().min(100_000)]);
        let _ = stream.flush();
        let _ = stream.read(&mut chunk);
    } else {
        let _ = stream.write_all(remaining);
        let _ = stream.flush();
    }
    let _ = stream.shutdown(Shutdown::Both);
}

type Events = Arc<Locked<Vec<DownloadProgress>>>;

struct Run {
    recovery: Arc<Locked<DownloadRecovery>>,
    events: Events,
}

async fn download(
    url: &Url,
    destination: &Path,
    resume_data: Option<Vec<u8>>,
) -> (Result<(), ExporterError>, Run) {
    let run = Run {
        recovery: Arc::default(),
        events: Arc::default(),
    };
    let (saved, events) = (run.recovery.clone(), run.events.clone());
    let result = ReqwestDownloader::new()
        .unwrap()
        .download(
            url,
            destination,
            resume_data,
            Arc::new(move |value| saved.set(value)),
            Arc::new(move |value| events.with(|all| all.push(value))),
            &CancellationToken::new(),
        )
        .await;
    (result, run)
}

/// Cancels a transfer once bytes have arrived and returns the recovery it reported.
async fn interrupt(url: &Url, destination: &Path) -> DownloadRecovery {
    let received = Arc::new(Locked::new(false));
    let saved = Arc::new(Locked::new(DownloadRecovery::default()));
    let cancel = CancellationToken::new();
    let task = {
        let (url, destination) = (url.clone(), destination.to_path_buf());
        let (received, saved, cancel) = (received.clone(), saved.clone(), cancel.clone());
        tokio::spawn(async move {
            ReqwestDownloader::new()
                .unwrap()
                .download(
                    &url,
                    &destination,
                    None,
                    Arc::new(move |value| saved.set(value)),
                    Arc::new(move |value: DownloadProgress| {
                        if value.transferred.bytes_received > 0 {
                            received.set(true);
                        }
                    }),
                    &cancel,
                )
                .await
        })
    };
    wait_until(|| received.current()).await;
    cancel.cancel();
    assert_eq!(task.await.unwrap(), Err(ExporterError::Cancelled));
    let recovery = saved.current();
    assert!(recovery.resume_data.is_some());
    recovery
}

#[tokio::test]
async fn persisted_token_resumes_with_a_new_downloader() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let recovery = interrupt(&server.url, &destination).await;
    let text = serde_json::to_string(&recovery).unwrap();
    let reloaded: DownloadRecovery = serde_json::from_str(&text).unwrap();
    server.finish_transfers(false);

    let (result, _) = download(&server.url, &destination, reloaded.resume_data).await;

    result.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload());
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains_key("range"));
    let range = &requests[1]["range"];
    assert_eq!(
        *range,
        format!("bytes={}-", recovery.progress.unwrap().bytes_received)
    );
    assert_eq!(requests[1]["if-range"], "\"synthetic-v1\"");
    assert!(requests.iter().all(|r| r["user-agent"] == USER_AGENT));
}

#[tokio::test]
async fn expired_original_url_falls_back_once_to_fresh_url() {
    let original = LocalHttpServer::start(payload());
    let fresh = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let saved = interrupt(&original.url, &destination).await;
    original.finish_transfers(true);
    fresh.finish_transfers(false);

    let (result, run) = download(&fresh.url, &destination, saved.resume_data).await;

    result.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload());
    assert_eq!(original.requests().len(), 2);
    assert_eq!(fresh.requests().len(), 1);
    assert!(
        original
            .requests()
            .iter()
            .chain(&fresh.requests())
            .all(|r| r["user-agent"] == USER_AGENT)
    );
    assert_eq!(run.recovery.current().resume_data, None);
    let events = run.events.current();
    assert_eq!(events[0].attempt, 0);
    let fallback: Vec<_> = events.iter().filter(|e| e.attempt == 1).collect();
    assert_eq!(fallback[0].transferred, TransferProgress::new(0, 0));
    let size = payload().len() as i64;
    assert_eq!(
        fallback.last().unwrap().transferred,
        TransferProgress::new(size, size)
    );
}

#[tokio::test]
async fn sends_only_the_player_identity() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(false);
    let directory = TemporaryDirectory::default();
    let (result, run) = download(&server.url, &directory.join("audio"), None).await;
    result.unwrap();
    assert_eq!(run.recovery.current().resume_data, None);
    let request = server.requests().pop().unwrap();
    assert_eq!(request["user-agent"], USER_AGENT);
    assert!(!request.contains_key("x-adp-token"));
}

#[tokio::test]
async fn changed_file_restarts_within_the_resume_attempt() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let saved = interrupt(&server.url, &destination).await;
    server
        .state
        .with(|state| state.etag = Some("\"synthetic-v2\"".into()));
    server.finish_transfers(false);

    let (result, run) = download(&server.url, &destination, saved.resume_data).await;

    result.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload());
    assert_eq!(server.requests().len(), 2);
    assert!(run.events.current().iter().all(|e| e.attempt == 0));
}

#[tokio::test]
async fn unusable_tokens_restart_at_the_supplied_url() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(false);
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");

    let (result, run) = download(&server.url, &destination, Some(b"not a token".to_vec())).await;

    result.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload());
    assert_eq!(server.requests().len(), 1);
    assert!(run.events.current().iter().any(|e| e.attempt == 1));
}

#[tokio::test]
async fn a_token_whose_bytes_are_gone_restarts() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let saved = interrupt(&server.url, &destination).await;
    std::fs::remove_file(&destination).unwrap();
    server.finish_transfers(false);

    let (result, _) = download(&server.url, &destination, saved.resume_data).await;

    result.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload());
    assert!(!server.requests()[1].contains_key("range"));
}

#[tokio::test]
async fn a_dropped_connection_keeps_what_arrived() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let task = {
        let (url, destination) = (server.url.clone(), destination.clone());
        tokio::spawn(async move { download(&url, &destination, None).await })
    };
    wait_until(|| server.requests().len() == 1).await;
    wait_until(|| std::fs::metadata(&destination).is_ok_and(|m| m.len() > 0)).await;
    server.drop_connections();

    let (result, run) = task.await.unwrap();

    assert!(
        matches!(result, Err(ExporterError::Download(_))),
        "{result:?}"
    );
    assert!(run.recovery.current().resume_data.is_some());
    assert!(destination.exists());
}

#[tokio::test]
async fn a_rejected_download_leaves_nothing_behind() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(true);
    let directory = TemporaryDirectory::default();
    let (result, run) = download(&server.url, &directory.join("book.aaxc"), None).await;
    assert_eq!(result, Err(ExporterError::Download("HTTP 403".into())));
    assert_eq!(run.recovery.current().resume_data, None);
    assert!(directory.contents().is_empty());
}

async fn cancelled_without_token(server: LocalHttpServer) {
    let directory = TemporaryDirectory::default();
    let destination = directory.join("book.aaxc");
    let cancel = CancellationToken::new();
    let saved = Arc::new(Locked::new(DownloadRecovery::default()));
    let task = {
        let (url, destination, cancel, saved) = (
            server.url.clone(),
            destination.clone(),
            cancel.clone(),
            saved.clone(),
        );
        tokio::spawn(async move {
            ReqwestDownloader::new()
                .unwrap()
                .download(
                    &url,
                    &destination,
                    None,
                    Arc::new(move |value| saved.set(value)),
                    Arc::new(|_| {}),
                    &cancel,
                )
                .await
        })
    };
    wait_until(|| std::fs::metadata(&destination).is_ok_and(|m| m.len() > 0)).await;
    cancel.cancel();
    assert_eq!(task.await.unwrap(), Err(ExporterError::Cancelled));
    assert_eq!(saved.current().resume_data, None);
    assert!(directory.contents().is_empty());
}

#[tokio::test]
async fn servers_without_a_validator_cannot_resume() {
    let server = LocalHttpServer::start(payload());
    server.state.with(|state| state.etag = None);
    cancelled_without_token(server).await;
}

#[tokio::test]
async fn weak_validators_cannot_resume() {
    let server = LocalHttpServer::start(payload());
    server
        .state
        .with(|state| state.etag = Some("W/\"weak\"".into()));
    cancelled_without_token(server).await;
}

#[tokio::test]
async fn servers_refusing_ranges_cannot_resume() {
    let server = LocalHttpServer::start(payload());
    server
        .state
        .with(|state| state.accept_ranges = "none".into());
    cancelled_without_token(server).await;
}

#[tokio::test]
async fn cancelling_before_starting_touches_nothing() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let reports = Arc::new(Locked::new(0));
    let counter = reports.clone();
    let result = ReqwestDownloader::new()
        .unwrap()
        .download(
            &Url::parse("http://127.0.0.1:9/").unwrap(),
            Path::new("/nonexistent/file"),
            None,
            Arc::new(move |_| counter.with(|count| *count += 1)),
            Arc::new(|_| {}),
            &cancel,
        )
        .await;
    assert_eq!(result, Err(ExporterError::Cancelled));
    assert_eq!(reports.current(), 0);
}

#[tokio::test]
async fn an_unwritable_destination_is_a_file_error() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(false);
    let directory = TemporaryDirectory::default();
    let (result, _) = download(&server.url, &directory.join("missing/book.aaxc"), None).await;
    assert!(
        matches!(&result, Err(ExporterError::FileSystem(detail)) if detail.starts_with("Could not save download")),
        "{result:?}"
    );
}
