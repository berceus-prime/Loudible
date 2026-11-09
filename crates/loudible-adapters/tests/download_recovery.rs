//! Fresh download, cancellation, and cleanup over a loopback HTTP server.

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

async fn download(url: &Url, destination: &Path) -> Result<(), ExporterError> {
    ReqwestDownloader::new()
        .unwrap()
        .download(
            url,
            destination,
            Arc::new(|_| {}),
            &CancellationToken::new(),
        )
        .await
}

#[tokio::test]
async fn downloads_from_zero_and_replaces_stale_bytes() {
    let body = payload();
    let server = LocalHttpServer::start(body.clone());
    server.finish_transfers(false);
    let directory = TemporaryDirectory::default();
    let path = directory.join("book.aaxc");
    std::fs::write(&path, b"stale partial").unwrap();
    download(&server.url, &path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert!(
        server
            .requests()
            .iter()
            .all(|request| !request.contains_key("range"))
    );
    assert_eq!(
        server.requests()[0].get("user-agent").map(String::as_str),
        Some("com.audible.playersdk.player/3.96.1 (Linux;Android 14) AndroidXMedia3/1.3.0")
    );
}

#[tokio::test]
async fn failed_status_discards_stale_bytes() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(true);
    let directory = TemporaryDirectory::default();
    let path = directory.join("book.aaxc");
    std::fs::write(&path, b"stale partial").unwrap();
    assert_eq!(
        download(&server.url, &path).await,
        Err(ExporterError::Download("HTTP 403".into()))
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn cancellation_discards_received_bytes() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let path = directory.join("book.aaxc");
    let cancel = CancellationToken::new();
    let task = {
        let (url, path, cancel) = (server.url.clone(), path.clone(), cancel.clone());
        tokio::spawn(async move {
            ReqwestDownloader::new()
                .unwrap()
                .download(&url, &path, Arc::new(|_| {}), &cancel)
                .await
        })
    };
    wait_until(|| std::fs::metadata(&path).is_ok_and(|m| m.len() > 0)).await;
    cancel.cancel();
    assert_eq!(task.await.unwrap(), Err(ExporterError::Cancelled));
    assert!(!path.exists());
}

#[tokio::test]
async fn dropped_connection_discards_received_bytes() {
    let server = LocalHttpServer::start(payload());
    let directory = TemporaryDirectory::default();
    let path = directory.join("book.aaxc");
    let task = {
        let (url, path) = (server.url.clone(), path.clone());
        tokio::spawn(async move { download(&url, &path).await })
    };
    wait_until(|| std::fs::metadata(&path).is_ok_and(|m| m.len() > 0)).await;
    server.drop_connections();
    assert!(matches!(
        task.await.unwrap(),
        Err(ExporterError::Download(_))
    ));
    assert!(!path.exists());
}

#[tokio::test]
async fn unwritable_destination_is_a_file_error() {
    let server = LocalHttpServer::start(payload());
    server.finish_transfers(false);
    let directory = TemporaryDirectory::default();
    let result = download(&server.url, &directory.join("missing/book.aaxc")).await;
    assert!(matches!(result, Err(ExporterError::FileSystem(_))));
}
