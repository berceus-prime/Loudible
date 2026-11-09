//! Doubles and helpers shared by the Rust tests, ported from `Tests/TestSupport`. All data is
//! synthetic: nothing here belongs to a real account.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta};
use futures::FutureExt;
use futures::future::BoxFuture;
use loudible_core::*;
use tempfile::TempDir;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

/// A lock-protected value for test doubles shared across tasks.
pub struct Locked<T>(Mutex<T>);

impl<T> Locked<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn with<R>(&self, body: impl FnOnce(&mut T) -> R) -> R {
        body(&mut self.0.lock().expect("no panic while holding the lock"))
    }

    pub fn set(&self, value: T) {
        self.with(|current| *current = value);
    }
}

impl<T: Clone> Locked<T> {
    pub fn current(&self) -> T {
        self.with(|value| value.clone())
    }
}

pub struct TestClock(Locked<Date>);

impl Default for TestClock {
    fn default() -> Self {
        Self(Locked::new(reference_date()))
    }
}

impl TestClock {
    pub fn advance(&self, seconds: i64) {
        self.0.with(|date| *date += TimeDelta::seconds(seconds));
    }
}

impl WallClock for TestClock {
    fn now(&self) -> Date {
        self.0.current()
    }
}

#[derive(Default)]
pub struct InMemoryCredentialStore {
    pub stored: Locked<Option<AudibleSession>>,
    pub fail_load: Locked<bool>,
    pub fail_save: Locked<bool>,
    pub fail_delete: Locked<bool>,
    pub save_count: Locked<usize>,
}

impl<T: Default> Default for Locked<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl InMemoryCredentialStore {
    pub fn with(session: AudibleSession) -> Self {
        let store = Self::default();
        store.stored.set(Some(session));
        store
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn load(&self) -> Result<Option<AudibleSession>, String> {
        if self.fail_load.current() {
            return Err("failure".into());
        }
        Ok(self.stored.current())
    }

    fn save(&self, session: &AudibleSession) -> Result<(), String> {
        if self.fail_save.current() {
            return Err("failure".into());
        }
        self.stored.set(Some(session.clone()));
        self.save_count.with(|count| *count += 1);
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        if self.fail_delete.current() {
            return Err("failure".into());
        }
        self.stored.set(None);
        Ok(())
    }
}

pub type Reply<T> = BoxFuture<'static, Result<T, ExporterError>>;
type Handler<A, T> = Locked<Arc<dyn Fn(A) -> Reply<T> + Send + Sync>>;

fn unscripted<A: 'static, T: 'static>() -> Arc<dyn Fn(A) -> Reply<T> + Send + Sync> {
    Arc::new(|_| async { Err(ExporterError::Unexpected("unscripted".into())) }.boxed())
}

fn handler<A, T, F, Fut>(body: F) -> Arc<dyn Fn(A) -> Reply<T> + Send + Sync>
where
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, ExporterError>> + Send + 'static,
{
    Arc::new(move |argument| body(argument).boxed())
}

pub struct LibraryRequest {
    pub number: usize,
    pub page_size: usize,
    pub marketplace: Marketplace,
    pub session: AudibleSession,
}

type MakeLogin = Arc<dyn Fn(Marketplace) -> Result<PendingLogin, ExporterError> + Send + Sync>;

/// Scriptable `AudibleApi`. Each handler defaults to failing so tests state what they use.
pub struct MockAudibleApi {
    make_login: Locked<MakeLogin>,
    register: Handler<(PendingLogin, String), AudibleSession>,
    refresh: Handler<AudibleSession, AudibleSession>,
    library: Handler<LibraryRequest, LibraryPage>,
    license: Handler<(String, AudibleSession), ContentLicense>,
    pub refresh_count: Locked<usize>,
    pub register_count: Locked<usize>,
    /// Pages requested from any store; `requested_marketplaces` has the store of each.
    pub requested_pages: Locked<Vec<usize>>,
    pub requested_marketplaces: Locked<Vec<Marketplace>>,
    pub requested_licenses: Locked<Vec<String>>,
    pub requested_license_marketplaces: Locked<Vec<Marketplace>>,
}

impl Default for MockAudibleApi {
    fn default() -> Self {
        Self {
            make_login: Locked::new(Arc::new(|marketplace| {
                Ok(pending_login(marketplace, login_id(0xAA), reference_date()))
            })),
            register: Locked::new(unscripted()),
            refresh: Locked::new(unscripted()),
            library: Locked::new(unscripted()),
            license: Locked::new(unscripted()),
            refresh_count: Locked::default(),
            register_count: Locked::default(),
            requested_pages: Locked::default(),
            requested_marketplaces: Locked::default(),
            requested_licenses: Locked::default(),
            requested_license_marketplaces: Locked::default(),
        }
    }
}

impl MockAudibleApi {
    pub fn on_make_login(
        &self,
        body: impl Fn(Marketplace) -> Result<PendingLogin, ExporterError> + Send + Sync + 'static,
    ) {
        self.make_login.set(Arc::new(body));
    }

    pub fn on_register<F, Fut>(&self, body: F)
    where
        F: Fn(PendingLogin, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AudibleSession, ExporterError>> + Send + 'static,
    {
        self.register
            .set(handler(move |(login, code)| body(login, code)));
    }

    pub fn on_refresh<F, Fut>(&self, body: F)
    where
        F: Fn(AudibleSession) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AudibleSession, ExporterError>> + Send + 'static,
    {
        self.refresh.set(handler(body));
    }

    pub fn on_library<F, Fut>(&self, body: F)
    where
        F: Fn(LibraryRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LibraryPage, ExporterError>> + Send + 'static,
    {
        self.library.set(handler(body));
    }

    pub fn on_license<F, Fut>(&self, body: F)
    where
        F: Fn(String, AudibleSession) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ContentLicense, ExporterError>> + Send + 'static,
    {
        self.license
            .set(handler(move |(asin, session)| body(asin, session)));
    }
}

#[async_trait]
impl AudibleApi for MockAudibleApi {
    fn make_login(&self, marketplace: Marketplace) -> Result<PendingLogin, ExporterError> {
        (self.make_login.current())(marketplace)
    }

    async fn register(
        &self,
        login: &PendingLogin,
        code: &str,
    ) -> Result<AudibleSession, ExporterError> {
        self.register_count.with(|count| *count += 1);
        (self.register.current())((login.clone(), code.to_string())).await
    }

    async fn refresh(&self, session: &AudibleSession) -> Result<AudibleSession, ExporterError> {
        self.refresh_count.with(|count| *count += 1);
        (self.refresh.current())(session.clone()).await
    }

    async fn library_page(
        &self,
        number: usize,
        page_size: usize,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<LibraryPage, ExporterError> {
        self.requested_pages.with(|pages| pages.push(number));
        self.requested_marketplaces
            .with(|stores| stores.push(marketplace));
        let request = LibraryRequest {
            number,
            page_size,
            marketplace,
            session: session.clone(),
        };
        (self.library.current())(request).await
    }

    async fn license(
        &self,
        asin: &str,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<ContentLicense, ExporterError> {
        self.requested_licenses
            .with(|asins| asins.push(asin.to_string()));
        self.requested_license_marketplaces
            .with(|stores| stores.push(marketplace));
        (self.license.current())((asin.to_string(), session.clone())).await
    }
}

/// `Task.checkCancellation()` for handlers.
pub fn check(cancel: &CancellationToken) -> Result<(), ExporterError> {
    if cancel.is_cancelled() {
        Err(ExporterError::Cancelled)
    } else {
        Ok(())
    }
}

pub fn write(path: &Path, contents: &str) -> Result<(), ExporterError> {
    std::fs::write(path, contents).map_err(|error| ExporterError::FileSystem(error.to_string()))
}

/// Downloads into a supplied path and reports synthetic progress.
pub struct MockDownloader {
    handler: Handler<(Url, PathBuf, CancellationToken), ()>,
    pub downloads: Locked<Vec<(Url, PathBuf)>>,
}

impl Default for MockDownloader {
    fn default() -> Self {
        Self {
            handler: Locked::new(handler(
                |(_, destination, _): (Url, PathBuf, _)| async move { write(&destination, "encrypted") },
            )),
            downloads: Locked::default(),
        }
    }
}

impl MockDownloader {
    pub fn on_download<F, Fut>(&self, body: F)
    where
        F: Fn(Url, PathBuf, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ExporterError>> + Send + 'static,
    {
        self.handler
            .set(handler(move |(url, path, cancel)| body(url, path, cancel)));
    }
}

#[async_trait]
impl FileDownloader for MockDownloader {
    async fn download(
        &self,
        url: &Url,
        destination: &Path,
        progress: DownloadProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        self.downloads
            .with(|all| all.push((url.clone(), destination.to_path_buf())));
        let report = |bytes| {
            progress(TransferProgress::new(bytes, 100));
        };
        report(50);
        let handler = self.handler.current();
        let result = handler((url.clone(), destination.to_path_buf(), cancel.clone())).await;
        if result.is_err() {
            let _ = std::fs::remove_file(destination);
            return result;
        }
        report(100);
        Ok(())
    }
}

/// Writes a fake M4B to the requested output unless the handler fails.
pub struct MockConverter {
    handler: Handler<(ConversionRequest, CancellationToken), ()>,
    pub requests: Locked<Vec<ConversionRequest>>,
}

impl Default for MockConverter {
    fn default() -> Self {
        Self {
            handler: Locked::new(handler(|(request, _): (ConversionRequest, _)| async move {
                write(&request.output, &format!("m4b:{}", request.metadata.asin))
            })),
            requests: Locked::default(),
        }
    }
}

impl MockConverter {
    pub fn on_convert<F, Fut>(&self, body: F)
    where
        F: Fn(ConversionRequest, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ExporterError>> + Send + 'static,
    {
        self.handler
            .set(handler(move |(request, cancel)| body(request, cancel)));
    }
}

#[async_trait]
impl AudioConverter for MockConverter {
    async fn convert(
        &self,
        request: ConversionRequest,
        progress: ConversionProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        self.requests.with(|all| all.push(request.clone()));
        progress(0.5);
        (self.handler.current())((request, cancel.clone())).await?;
        progress(1.0);
        Ok(())
    }
}

/// Suspends callers until `open()` is called. Lets tests hold a mocked operation mid-flight.
#[derive(Clone)]
pub struct AsyncGate(Arc<watch::Sender<bool>>);

impl Default for AsyncGate {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(false)))
    }
}

impl AsyncGate {
    pub async fn wait(&self) {
        let _ = self.0.subscribe().wait_for(|is_open| *is_open).await;
    }

    pub fn open(&self) {
        self.0.send_replace(true);
    }
}

/// Polls `condition` until it holds; panics after five seconds instead of hanging the test run.
#[track_caller]
pub fn wait_until(condition: impl Fn() -> bool) -> impl Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "condition not met at {caller}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

/// A uniquely named directory, removed when dropped.
pub struct TemporaryDirectory(TempDir);

impl Default for TemporaryDirectory {
    fn default() -> Self {
        Self(TempDir::new().expect("create a temporary directory"))
    }
}

impl TemporaryDirectory {
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    pub fn contents(&self) -> Vec<String> {
        contents_of(self.0.path())
    }
}

/// Everything in `directory`, hidden files included, sorted; empty if it doesn't exist.
pub fn contents_of(directory: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!("cannot read directory {}: {error}", directory.display()),
    };
    let mut names: Vec<String> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("cannot read entry in {}: {error}", directory.display())
                })
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn missing_directory_is_empty_and_existing_entries_are_sorted() {
        let directory = TemporaryDirectory::default();
        assert!(contents_of(&directory.join("missing")).is_empty());
        std::fs::write(directory.join("z"), "").unwrap();
        std::fs::write(directory.join(".hidden"), "").unwrap();
        assert_eq!(contents_of(directory.path()), [".hidden", "z"]);
    }

    #[test]
    #[should_panic(expected = "cannot read directory")]
    fn a_file_is_not_treated_as_an_empty_directory() {
        let directory = TemporaryDirectory::default();
        let file = directory.join("file");
        std::fs::write(&file, "").unwrap();
        contents_of(&file);
    }
}

// MARK: Fixtures

pub fn reference_date() -> Date {
    DateTime::from_timestamp(1_800_000_000, 0).expect("in range")
}

pub fn seconds_after_reference(seconds: i64) -> Date {
    reference_date() + TimeDelta::seconds(seconds)
}

pub fn device() -> DeviceIdentity {
    DeviceIdentity::new("0123456789ABCDEF0123456789ABCDEF")
}

pub fn session() -> AudibleSession {
    session_in(Marketplace::US)
}

pub fn session_in(marketplace: Marketplace) -> AudibleSession {
    AudibleSession {
        marketplace,
        device: device(),
        customer_id: "amzn1.account.SYNTHETIC".into(),
        customer_name: Some("Test User".into()),
        access_token: "Atna|synthetic-access".into(),
        access_token_expires_at: seconds_after_reference(3600),
        refresh_token: "Atnr|synthetic-refresh".into(),
        adp_token: "{enc:synthetic}{key:synthetic}".into(),
        device_private_key:
            "-----BEGIN RSA PRIVATE KEY-----\nsynthetic\n-----END RSA PRIVATE KEY-----\n".into(),
    }
}

pub fn session_with_token(token: &str) -> AudibleSession {
    AudibleSession {
        access_token: token.into(),
        ..session()
    }
}

pub fn expired_session() -> AudibleSession {
    AudibleSession {
        access_token_expires_at: DateTime::UNIX_EPOCH,
        ..session()
    }
}

pub fn login_id(number: u128) -> Uuid {
    Uuid::from_u128(number)
}

pub fn pending_login(marketplace: Marketplace, id: Uuid, created_at: Date) -> PendingLogin {
    PendingLogin {
        id,
        marketplace,
        device: device(),
        code_verifier: "synthetic-verifier".into(),
        login_url: Url::parse(&format!(
            "https://{}/ap/signin?synthetic=1",
            marketplace.amazon_host()
        ))
        .expect("valid URL"),
        cookies: vec![LoginCookie {
            name: "frc".into(),
            value: "x".into(),
            domain: format!(".amazon.{}", marketplace.domain),
        }],
        user_agent: None,
        created_at,
    }
}

/// The sign-in redirect carrying `code`, a raw authorization code that is encoded here.
pub fn callback_url(marketplace: Marketplace, code: &str) -> String {
    let mut url = Url::parse(&format!(
        "https://{}{}",
        marketplace.amazon_host(),
        LoginCallback::PATH
    ))
    .expect("a valid callback URL");
    url.query_pairs_mut()
        .append_pair("openid.assoc_handle", "amzn_audible_ios_us")
        .append_pair(LoginCallback::AUTHORIZATION_CODE_PARAMETER, code);
    url.into()
}

pub fn us_callback() -> String {
    callback_url(Marketplace::US, "synthetic-code")
}

pub fn book(asin: &str) -> Audiobook {
    book_in(asin, &format!("Book {asin}"), Marketplace::US)
}

pub fn book_in(asin: &str, title: &str, marketplace: Marketplace) -> Audiobook {
    Audiobook {
        authors: vec!["Author A".into()],
        narrators: vec!["Narrator N".into()],
        ..Audiobook::new(asin, marketplace, title)
    }
}

pub fn entry(asin: &str) -> LibraryEntry {
    entry_of(book(asin), "SinglePartBook", "Product", false, vec![])
}

pub fn entry_in(asin: &str, marketplace: Marketplace) -> LibraryEntry {
    entry_of(
        book_in(asin, &format!("Book {asin}"), marketplace),
        "SinglePartBook",
        "Product",
        false,
        vec![],
    )
}

pub fn entry_of(
    book: Audiobook,
    delivery_type: &str,
    content_type: &str,
    is_ayce: bool,
    relationships: Vec<ProductRelationship>,
) -> LibraryEntry {
    LibraryEntry {
        book,
        content_delivery_type: Some(delivery_type.into()),
        content_type: Some(content_type.into()),
        is_ayce: Some(is_ayce),
        relationships,
    }
}

pub fn license(asin: &str) -> ContentLicense {
    ContentLicense {
        asin: asin.into(),
        decryption: Decryption::Aaxc {
            key: vec![0x11; 16],
            iv: vec![0x22; 16],
        },
        download_url: Url::parse(&format!("https://cdn.example.invalid/{asin}.aaxc"))
            .expect("valid URL"),
        chapters: vec![Chapter {
            title: "Chapter 1".into(),
            start_milliseconds: 0,
            length_milliseconds: 60_000,
        }],
        runtime_milliseconds: Some(60_000),
        codec: Some("mp4a.40.2".into()),
        content_format: Some("AAX_44_128".into()),
    }
}
