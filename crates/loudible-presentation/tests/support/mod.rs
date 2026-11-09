//! Fixtures shared by the view-model tests. All data is synthetic.
#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;

use loudible_core::*;
use loudible_presentation::*;
use loudible_test_support::*;

/// Runs `body` where the models can live: one thread, with a `LocalSet`.
pub fn local<F: Future>(body: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, body)
}

pub fn page(entries: Vec<LibraryEntry>) -> LibraryPage {
    LibraryPage {
        entries,
        total_count: None,
    }
}

pub fn titled(asin: &str, title: &str) -> LibraryEntry {
    entry_of(
        book_in(asin, title, Marketplace::US),
        "SinglePartBook",
        "Product",
        false,
        vec![],
    )
}

pub struct Fixture {
    pub api: Arc<MockAudibleApi>,
    pub store: Arc<InMemoryCredentialStore>,
    pub converter: Arc<MockConverter>,
    pub downloader: Arc<MockDownloader>,
    pub work: TemporaryDirectory,
    pub downloads: LocalDownloads,
}

impl Fixture {
    /// Signed in, with B1 and B2 in the US library and nothing elsewhere.
    pub fn new() -> Self {
        let work = TemporaryDirectory::default();
        let downloads = LocalDownloads::with_trash(
            work.join("Downloads"),
            work.join("Manifests"),
            Arc::new(|path| std::fs::remove_file(path)),
        );
        let fixture = Self {
            api: Arc::default(),
            store: Arc::new(InMemoryCredentialStore::with(session())),
            converter: Arc::default(),
            downloader: Arc::default(),
            work,
            downloads,
        };
        fixture.license_succeeds();
        fixture.api.on_library(|request| async move {
            Ok(page(if request.marketplace == Marketplace::US {
                vec![entry("B1"), entry("B2")]
            } else {
                vec![]
            }))
        });
        fixture
    }

    pub fn license_succeeds(&self) {
        self.api
            .on_license(|asin, _| async move { Ok(license(&asin)) });
    }

    pub fn auth(&self) -> AuthenticationCoordinator {
        AuthenticationCoordinator::new(
            self.api.clone(),
            self.store.clone(),
            Arc::new(TestClock::default()),
        )
    }

    pub fn dependencies(&self, _slots: usize) -> AppDependencies {
        let auth = self.auth();
        AppDependencies {
            library: LibraryService::new(
                self.api.clone(),
                auth.clone(),
                LibraryService::DEFAULT_PAGE_SIZE,
            ),
            queue: ExportQueue::new(
                auth.clone(),
                self.api.clone(),
                self.downloader.clone(),
                self.converter.clone(),
                self.downloads.clone(),
            ),
            auth,
            downloads: self.downloads.clone(),
            export_folder: self.downloads.folder.clone(),
        }
    }

    /// A download as an earlier run left it: the audio file and its manifest.
    pub fn keep_download(&self, book: &Audiobook) {
        self.downloads.ensure_folder().unwrap();
        let name = format!("{} [{}].aaxc", book.title, book.asin);
        std::fs::write(self.downloads.folder.join(name), "enc").unwrap();
        let manifest = DownloadManifest::new(
            book.clone(),
            license(&book.asin).decryption,
            vec![],
            reference_date(),
        );
        self.downloads.write_manifest(&manifest, None).unwrap();
    }

    pub async fn make_app(&self) -> (AppViewModel, ExportQueue) {
        self.app_with(1).await
    }

    pub async fn make_app_with_two_slots(&self) -> (AppViewModel, ExportQueue) {
        self.app_with(2).await
    }

    async fn app_with(&self, slots: usize) -> (AppViewModel, ExportQueue) {
        let dependencies = self.dependencies(slots);
        let queue = dependencies.queue.clone();
        let changes = Changes::default();
        let app = AppViewModel::new(
            SessionViewModel::new(dependencies.auth, changes.clone()),
            LibraryViewModel::new(dependencies.library, changes.clone()),
            ExportViewModel::new(queue.clone(), dependencies.export_folder, changes.clone()),
            dependencies.downloads,
            changes,
        );
        app.start().await;
        (app, queue)
    }
}
