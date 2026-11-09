use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loudible_core::{
    Audiobook, BookAction, BookMenuItem, BookStatus, ExportJob, LibrarySelection, Marketplace,
    PendingLogin,
};

use crate::app_view_model::{DiskState, MenuFacts, offers};
use crate::library_view_model::marketplace_badge;
use crate::{AppViewModel, LoadState, MarketplaceCount, Notice};

/// Everything the window draws, frozen at one moment. The UI thread only ever reads these.
#[derive(Clone, Debug)]
pub struct AppSnapshot {
    pub session: SessionSnapshot,
    pub library: LibrarySnapshot,
    pub export: ExportSnapshot,
    pub notices: Vec<Notice>,
    pub has_active_jobs: bool,
    disk: DiskState,
}

#[derive(Clone, Debug)]
pub struct SessionSnapshot {
    pub marketplace: Marketplace,
    pub pending_login: Option<PendingLogin>,
    pub pasted_callback_url: String,
    pub is_working: bool,
    pub error_message: Option<String>,
    pub has_restored: bool,
    pub is_signed_in: bool,
    pub needs_reauthentication: bool,
    pub is_registering: bool,
    pub can_sign_in: bool,
    pub can_sign_out: bool,
    pub can_change_marketplace: bool,
    pub can_submit_pasted_url: bool,
    pub account_description: Option<String>,
    pub customer_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LibrarySnapshot {
    pub book_count: usize,
    /// The books matching the search, in order.
    pub visible_books: Arc<Vec<Audiobook>>,
    /// The status of each of `visible_books`, in the same order.
    pub statuses: Vec<BookStatus>,
    pub load_state: LoadState,
    pub is_loading: bool,
    pub is_checking_marketplaces: bool,
    pub home_marketplace: Option<Marketplace>,
    pub other_marketplaces: Vec<MarketplaceCount>,
    pub marketplace_notice: Option<String>,
    pub selection: LibrarySelection,
    pub search_text: String,
    pub error_message: Option<String>,
    pub can_refresh: bool,
    pub can_select: bool,
    pub summary: String,
    /// Selected books the toolbar's Download button acts on.
    pub selected_for_download: usize,
    /// Selected books the toolbar's Export button acts on.
    pub selected_for_export: usize,
    remote_asins: Arc<HashSet<String>>,
}

#[derive(Clone, Debug)]
pub struct ExportSnapshot {
    pub export_folder: PathBuf,
    pub jobs: Vec<ExportJob>,
    pub is_running: bool,
    pub is_waiting_for_authentication: bool,
    pub active_job_count: usize,
    pub notice: Option<&'static str>,
    pub error_message: Option<String>,
    pub has_work_suspend_loses: bool,
}

impl AppSnapshot {
    /// Reads the models. Statuses are looked up per book, so a large library doesn't multiply
    /// with the number of jobs on every progress update.
    pub fn capture(app: &AppViewModel) -> Self {
        let (session, library, export) = (app.session(), app.library(), app.export());
        let disk = app.disk();
        let jobs = export.jobs();
        let visible_books = library.visible_books();

        let mut jobs_by_book: HashMap<&str, Vec<ExportJob>> = HashMap::new();
        for job in &jobs {
            jobs_by_book
                .entry(&job.book.asin)
                .or_default()
                .push(job.clone());
        }
        let failures = export.failures();
        let statuses = visible_books
            .iter()
            .map(|book| {
                let asin = book.asin.as_str();
                BookStatus::resolve(
                    asin,
                    jobs_by_book.get(asin).map_or(&[], Vec::as_slice),
                    failures
                        .get(asin)
                        .map(|(kind, text)| (*kind, text.as_str())),
                    disk.downloaded_files.contains_key(asin),
                    disk.exported_files.contains_key(asin),
                )
            })
            .collect();

        let remote_asins = library.remote_asins();
        let is_signed_in = session.is_signed_in();
        let facts = MenuFacts {
            jobs: &jobs,
            disk: &disk,
            is_signed_in,
        };
        let (mut selected_for_download, mut selected_for_export) = (0, 0);
        for book in library.selected_books() {
            let items = facts.items(&book.asin, remote_asins.contains(&book.asin));
            selected_for_download += usize::from(offers(&items, BookAction::Download));
            selected_for_export += usize::from(offers(&items, BookAction::Export));
        }

        Self {
            session: SessionSnapshot {
                marketplace: session.marketplace(),
                pending_login: session.pending_login(),
                pasted_callback_url: session.pasted_callback_url(),
                is_working: session.is_working(),
                error_message: session.error_message(),
                has_restored: session.has_restored(),
                is_signed_in,
                needs_reauthentication: session.needs_reauthentication(),
                is_registering: session.is_registering(),
                can_sign_in: session.can_sign_in(),
                can_sign_out: session.can_sign_out(),
                can_change_marketplace: session.can_change_marketplace(),
                can_submit_pasted_url: session.can_submit_pasted_url(),
                account_description: session.account_description(),
                customer_name: session.session().and_then(|session| session.customer_name),
            },
            library: LibrarySnapshot {
                book_count: library.books().len(),
                visible_books,
                statuses,
                load_state: library.load_state(),
                is_loading: library.is_loading(),
                is_checking_marketplaces: library.is_checking_marketplaces(),
                home_marketplace: library.home_marketplace(),
                other_marketplaces: library.other_marketplaces(),
                marketplace_notice: library.marketplace_notice(),
                selection: library.selection(),
                search_text: library.search_text(),
                error_message: library.error_message(),
                can_refresh: library.can_refresh(),
                can_select: library.can_select(),
                summary: library.summary(),
                selected_for_download,
                selected_for_export,
                remote_asins,
            },
            export: ExportSnapshot {
                export_folder: export.export_folder().to_path_buf(),
                is_running: export.is_running(),
                is_waiting_for_authentication: export.is_waiting_for_authentication(),
                active_job_count: export.active_job_count(),
                notice: export.notice(),
                error_message: export.error_message(),
                has_work_suspend_loses: export.has_work_suspend_loses(),
                jobs,
            },
            notices: app.errors().notices(),
            has_active_jobs: app.has_active_jobs(),
            disk,
        }
    }

    /// Worked out when a menu opens rather than for every book on every update.
    pub fn menu_items(&self, book: &Audiobook) -> Vec<BookMenuItem> {
        let facts = MenuFacts {
            jobs: &self.export.jobs,
            disk: &self.disk,
            is_signed_in: self.session.is_signed_in,
        };
        facts.items(&book.asin, self.library.remote_asins.contains(&book.asin))
    }

    /// Whether the book was in the last refreshed library.
    pub fn is_in_library(&self, asin: &str) -> bool {
        self.library.remote_asins.contains(asin)
    }

    pub fn marketplace_badge(&self, book: &Audiobook) -> Option<String> {
        marketplace_badge(self.library.home_marketplace, book)
    }

    pub fn downloaded_file(&self, asin: &str) -> Option<&Path> {
        self.disk.downloaded_files.get(asin).map(PathBuf::as_path)
    }

    pub fn exported_file(&self, asin: &str) -> Option<&Path> {
        self.disk.exported_files.get(asin).map(PathBuf::as_path)
    }
}
