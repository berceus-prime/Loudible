//! Everything external the application needs, behind traits that tests replace.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    AudibleSession, Audiobook, Chapter, ContentLicense, Date, Decryption, ExporterError,
    LibraryPage, Marketplace, PendingLogin,
};

/// Audible operations the application needs. Implemented over HTTP by the adapters layer.
#[async_trait]
pub trait AudibleApi: Send + Sync {
    /// Creates a new device identity, PKCE verifier, login URL, and initial cookies.
    fn make_login(&self, marketplace: Marketplace) -> Result<PendingLogin, ExporterError>;
    /// Registers the device using the authorization code from the login callback.
    async fn register(
        &self,
        login: &PendingLogin,
        authorization_code: &str,
    ) -> Result<AudibleSession, ExporterError>;
    /// Returns the session with a fresh access token.
    async fn refresh(&self, session: &AudibleSession) -> Result<AudibleSession, ExporterError>;
    /// Fetches one 1-based page of the library held in `marketplace`. The same device
    /// registration is accepted by every store, so it need not be the session's own.
    async fn library_page(
        &self,
        number: usize,
        page_size: usize,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<LibraryPage, ExporterError>;
    /// Requests and decrypts a download license from the store that holds the title.
    async fn license(
        &self,
        asin: &str,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<ContentLicense, ExporterError>;
}

/// Persists the signed-in session. Errors are technical descriptions.
pub trait CredentialStore: Send + Sync {
    fn load(&self) -> Result<Option<AudibleSession>, String>;
    fn save(&self, session: &AudibleSession) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// How much of a download has arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferProgress {
    pub bytes_received: i64,
    pub bytes_expected: i64,
}

impl TransferProgress {
    pub fn new(bytes_received: i64, bytes_expected: i64) -> Self {
        Self {
            bytes_received,
            bytes_expected,
        }
    }

    /// 0...1; 0 while the size is unknown.
    pub fn fraction(&self) -> f64 {
        if self.bytes_expected > 0 {
            (self.bytes_received as f64 / self.bytes_expected as f64).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

pub type DownloadProgressSink = Arc<dyn Fn(TransferProgress) + Send + Sync>;
pub type ConversionProgressSink = Arc<dyn Fn(f64) + Send + Sync>;

/// Downloads a URL completely to `destination`, starting from byte zero.
#[async_trait]
pub trait FileDownloader: Send + Sync {
    async fn download(
        &self,
        url: &Url,
        destination: &Path,
        progress: DownloadProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError>;
}

/// Metadata written into the exported file in addition to what the source carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookMetadata {
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
    pub asin: String,
}

impl From<&Audiobook> for BookMetadata {
    fn from(book: &Audiobook) -> Self {
        Self {
            title: book.title.clone(),
            subtitle: book.subtitle.clone(),
            authors: book.authors.clone(),
            narrators: book.narrators.clone(),
            publisher: book.publisher.clone(),
            release_date: book.release_date.clone(),
            asin: book.asin.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub decryption: Decryption,
    pub chapters: Vec<Chapter>,
    pub metadata: BookMetadata,
}

/// Decrypts and remuxes a downloaded file into an M4B at `request.output`. Success replaces
/// `request.output`; failure leaves it, and `request.input`, as they were.
#[async_trait]
pub trait AudioConverter: Send + Sync {
    async fn convert(
        &self,
        request: ConversionRequest,
        progress: ConversionProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError>;
}

pub trait WallClock: Send + Sync {
    fn now(&self) -> Date;
}

pub struct SystemClock;

impl WallClock for SystemClock {
    fn now(&self) -> Date {
        Utc::now()
    }
}
