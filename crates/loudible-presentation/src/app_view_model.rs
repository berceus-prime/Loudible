use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use loudible_core::{
    Audiobook, BookAction, BookMenuContext, BookMenuItem, BookStatus, DeviceIdentity, ExportJob,
    LocalDownloads, book_menu_items,
};
use url::Url;

use crate::{Changes, ErrorToastQueue, ExportViewModel, LibraryViewModel, SessionViewModel};

/// What the folders on disk held at the last rescan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiskState {
    pub downloaded_files: HashMap<String, PathBuf>,
    pub exported_files: HashMap<String, PathBuf>,
    pub manifest_asins: HashSet<String>,
}

pub(crate) struct MenuFacts<'a> {
    pub jobs: &'a [ExportJob],
    pub disk: &'a DiskState,
    pub is_signed_in: bool,
}

impl MenuFacts<'_> {
    pub fn items(&self, asin: &str, is_in_library: bool) -> Vec<BookMenuItem> {
        book_menu_items(
            asin,
            self.jobs,
            &BookMenuContext {
                is_downloaded: self.disk.downloaded_files.contains_key(asin),
                is_exported: self.disk.exported_files.contains_key(asin),
                is_in_library,
                has_manifest: self.disk.manifest_asins.contains(asin),
                is_signed_in: self.is_signed_in,
            },
        )
    }
}

pub(crate) fn offers(items: &[BookMenuItem], action: BookAction) -> bool {
    items
        .iter()
        .any(|item| item.is_enabled && item.action == action)
}

struct Inner {
    session: SessionViewModel,
    library: LibraryViewModel,
    export: ExportViewModel,
    errors: Rc<ErrorToastQueue>,
    downloads: LocalDownloads,
    changes: Changes,
    disk: RefCell<DiskState>,
    last_session_device: RefCell<Option<DeviceIdentity>>,
    started: Cell<bool>,
}

/// Connects the session, library, and export models: signing in loads the library,
/// signing out clears it, and a new sign-in resumes jobs paused for authentication. Also keeps
/// track of downloaded and exported books from the files on disk.
#[derive(Clone)]
pub struct AppViewModel(Rc<Inner>);

/// Runs file work off the model thread. A panic there leaves the previous state in place.
async fn on_blocking_thread<T: Send + 'static>(
    body: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    tokio::task::spawn_blocking(body).await.ok()
}

impl AppViewModel {
    pub fn new(
        session: SessionViewModel,
        library: LibraryViewModel,
        export: ExportViewModel,
        downloads: LocalDownloads,
        changes: Changes,
    ) -> Self {
        let errors = Rc::new(ErrorToastQueue::new(changes.clone()));
        let sink = errors.clone();
        session.set_on_error(move |error| sink.present(error));
        let sink = errors.clone();
        export.set_on_error(move |error| sink.present(error));
        Self(Rc::new(Inner {
            session,
            library,
            export,
            errors,
            downloads,
            changes,
            disk: RefCell::default(),
            last_session_device: RefCell::default(),
            started: Cell::new(false),
        }))
    }

    pub fn session(&self) -> &SessionViewModel {
        &self.0.session
    }

    pub fn library(&self) -> &LibraryViewModel {
        &self.0.library
    }

    pub fn export(&self) -> &ExportViewModel {
        &self.0.export
    }

    pub fn errors(&self) -> &ErrorToastQueue {
        &self.0.errors
    }

    pub async fn start(&self) {
        if self.0.started.replace(true) {
            return;
        }
        // Old staging files and recovery records cannot be resumed.
        let downloads = self.0.downloads.clone();
        on_blocking_thread(move || downloads.sweep_partials()).await;
        let weak = Rc::downgrade(&self.0);
        self.0.export.set_on_job_finished(move || {
            if let Some(inner) = weak.upgrade() {
                tokio::task::spawn_local(async move { Self(inner).rescan().await });
            }
        });
        self.0.export.start();
        self.0.session.start().await;
        self.session_changed().await;
        self.rescan().await;
        self.observe_session();
    }

    fn observe_session(&self) {
        let weak = Rc::downgrade(&self.0);
        let changes = self.0.changes.clone();
        tokio::task::spawn_local(async move {
            loop {
                changes.session_changed().await;
                let Some(inner) = weak.upgrade() else { break };
                Self(inner).session_changed().await;
            }
        });
    }

    /// Reacts to sign-in state; public so tests can drive it deterministically.
    pub async fn session_changed(&self) {
        let (session, library, export) = (&self.0.session, &self.0.library, &self.0.export);
        library.set_signed_in(session.is_signed_in());
        let device = session.session().map(|session| session.device);
        let previous = self.0.last_session_device.replace(device.clone());

        if !session.is_signed_in() {
            if !session.needs_reauthentication() && previous.is_some() {
                library.reset();
            }
            return;
        }
        if export.is_waiting_for_authentication() {
            export.resume_after_authentication();
        }
        // A restored or new sign-in loads the library; token refreshes keep the device and do not.
        if device != previous {
            self.refresh_library().await;
        }
    }

    /// Reloads the library and downloaded books shown with it.
    pub async fn refresh_library(&self) {
        self.0.library.refresh().await;
        self.rescan().await;
    }

    // MARK: Files on disk

    /// Rereads the download and export folders. Manifests whose audio file is gone are deleted,
    /// except for books with a job in progress (a download writes its manifest before the file
    /// gets its final name).
    pub async fn rescan(&self) {
        let downloads = self.0.downloads.clone();
        let export_folder = self.0.export.export_folder().to_path_buf();
        let mut keeping = self.0.export.busy_asins();
        let result = on_blocking_thread(move || {
            let downloaded_files = downloads.scan_downloads();
            keeping.extend(downloaded_files.keys().cloned());
            downloads.remove_orphaned_manifests(&keeping);
            let manifests = downloads.manifests();
            let exported_files = downloads.scan_exports(&export_folder);
            let local_books: Vec<Audiobook> = manifests
                .iter()
                .filter(|manifest| downloaded_files.contains_key(&manifest.book.asin))
                .map(|manifest| {
                    let mut book = manifest.book.clone();
                    // The stored cover still shows once Audible's image URL is gone.
                    if let Some(cover) = downloads
                        .stored_cover(&book.asin)
                        .and_then(|path| Url::from_file_path(path).ok())
                    {
                        book.cover_image_url = Some(cover);
                    }
                    book
                })
                .collect();
            let manifest_asins = manifests.into_iter().map(|m| m.book.asin).collect();
            (
                DiskState {
                    downloaded_files,
                    exported_files,
                    manifest_asins,
                },
                local_books,
            )
        })
        .await;
        let Some((disk, local_books)) = result else {
            return;
        };
        *self.0.disk.borrow_mut() = disk;
        self.0.changes.mark();
        self.0.library.apply_local(local_books);
    }

    pub(crate) fn disk(&self) -> DiskState {
        self.0.disk.borrow().clone()
    }

    pub fn downloaded_files(&self) -> HashMap<String, PathBuf> {
        self.0.disk.borrow().downloaded_files.clone()
    }

    pub fn exported_files(&self) -> HashMap<String, PathBuf> {
        self.0.disk.borrow().exported_files.clone()
    }

    // MARK: Per-book state

    pub fn status(&self, book: &Audiobook) -> BookStatus {
        let disk = self.0.disk.borrow();
        let asin = book.asin.as_str();
        let failure = self.0.export.failure(asin);
        BookStatus::resolve(
            asin,
            &self.0.export.jobs(),
            failure.as_ref().map(|(kind, text)| (*kind, text.as_str())),
            disk.downloaded_files.contains_key(asin),
            disk.exported_files.contains_key(asin),
        )
    }

    pub fn menu_items(&self, book: &Audiobook) -> Vec<BookMenuItem> {
        let disk = self.0.disk.borrow();
        let jobs = self.0.export.jobs();
        let facts = MenuFacts {
            jobs: &jobs,
            disk: &disk,
            is_signed_in: self.0.session.is_signed_in(),
        };
        facts.items(&book.asin, self.0.library.is_in_library(&book.asin))
    }

    /// Runs the queue-backed menu actions. Showing files and the delete confirmation are up to
    /// the view.
    pub async fn perform(&self, action: BookAction, book: &Audiobook) {
        let export = &self.0.export;
        match action {
            BookAction::Download => {
                export.download(std::slice::from_ref(book));
            }
            BookAction::Cancel { job_id, .. } => export.cancel(job_id),
            BookAction::Export => export.export(std::slice::from_ref(book)),
            BookAction::DeleteDownload => self.delete_download(book).await,
            BookAction::ShowDownload | BookAction::ShowExport => {}
        }
    }

    // MARK: Selection

    /// The selected books whose menu offers `action` enabled, i.e. the ones the toolbar's
    /// Download or Export button acts on.
    pub fn selected_books_offering(&self, action: BookAction) -> Vec<Audiobook> {
        self.0
            .library
            .selected_books()
            .into_iter()
            .filter(|book| offers(&self.menu_items(book), action))
            .collect()
    }

    /// Downloads or exports the selected books that offer it, as their menus would, then clears
    /// the selection.
    pub fn perform_on_selection(&self, action: BookAction) {
        let books = self.selected_books_offering(action);
        if books.is_empty() {
            return;
        }
        match action {
            BookAction::Download => self.0.export.download(&books),
            BookAction::Export => self.0.export.export(&books),
            _ => return,
        }
        self.0.library.clear_selection();
    }

    /// Moves the book's audio file to the Trash and deletes its manifest and any unfinished
    /// download; the M4B is kept.
    pub async fn delete_download(&self, book: &Audiobook) {
        let downloads = self.0.downloads.clone();
        let asin = book.asin.clone();
        let result = on_blocking_thread(move || downloads.trash_download(&asin)).await;
        if let Some(Err(error)) = result {
            self.0.export.set_error_message(Some(error.to_string()));
        }
        self.rescan().await;
    }

    pub fn has_active_jobs(&self) -> bool {
        self.0.export.active_job_count() > 0
    }

    /// Stops every job, then signs out. Incomplete downloads are discarded; completed files stay.
    pub async fn sign_out(&self) {
        self.0.export.suspend().await;
        self.0.session.sign_out();
        self.rescan().await;
    }
}
