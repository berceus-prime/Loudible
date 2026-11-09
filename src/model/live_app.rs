//! The real engine behind the window: reads `AppSnapshot`s, translates them into the view types
//! once per snapshot, and sends everything the user does as a `Command`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use loudible_core as core;
use loudible_presentation::{self as presentation, AppHandle, AppSnapshot, Command};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::app_error::{AppError, CredentialOperation};
use super::app_model::{AppModel, ModelEffect};
use super::book::{
    Asin, Audiobook, BookAction, BookMenuItem, BookStatus, BookTileState, JobKind, Marketplace,
    TransferProgress,
};
use super::mock_app::{PendingLogin, Toast};
use super::preview_data::Account;
use crate::shell::browser::BrowserPage;

pub struct LiveApp {
    handle: AppHandle,
    snapshot: Arc<AppSnapshot>,
    /// Rebuilt only when the search or library changes, not on every progress update.
    books_source: Arc<Vec<core::Audiobook>>,
    books: Vec<Audiobook>,
    positions: HashMap<Asin, usize>,
    tiles: Vec<BookTileState>,
    account: Option<Account>,
    pending_login: Option<PendingLogin>,
    browser_page: Option<BrowserPage>,
    toasts: Vec<Toast>,
    toast_ids: HashMap<Uuid, u64>,
    next_toast_id: u64,
    copied_toasts: HashSet<u64>,
    messages: Vec<String>,
    notices: Vec<String>,
    others: Vec<(Marketplace, usize)>,
    suspending: Option<oneshot::Receiver<()>>,
    has_suspended: bool,
}

fn job_kind(kind: core::JobKind) -> JobKind {
    match kind {
        core::JobKind::Download => JobKind::Download,
        core::JobKind::Export => JobKind::Export,
    }
}

fn transfer(progress: core::TransferProgress) -> TransferProgress {
    TransferProgress {
        received: progress.bytes_received.max(0) as u64,
        expected: progress.bytes_expected.max(0) as u64,
    }
}

fn status(status: &core::BookStatus) -> BookStatus {
    match status {
        core::BookStatus::None => BookStatus::None,
        core::BookStatus::Queued(kind) => BookStatus::Queued(job_kind(*kind)),
        core::BookStatus::Running {
            kind,
            progress,
            transferred,
        } => BookStatus::Running {
            kind: job_kind(*kind),
            progress: *progress as f32,
            transferred: transferred.map(transfer),
        },
        core::BookStatus::Failed { kind, message } => BookStatus::Failed {
            kind: job_kind(*kind),
            message: message.clone(),
        },
        core::BookStatus::Downloaded => BookStatus::Downloaded,
        core::BookStatus::Exported => BookStatus::Exported,
    }
}

fn action(action: core::BookAction) -> BookAction {
    match action {
        core::BookAction::Download => BookAction::Download,
        core::BookAction::Cancel { kind, .. } => BookAction::Cancel(job_kind(kind)),
        core::BookAction::Export => BookAction::Export,
        core::BookAction::ShowDownload => BookAction::ShowDownload,
        core::BookAction::ShowExport => BookAction::ShowExport,
        core::BookAction::DeleteDownload => BookAction::DeleteDownload,
    }
}

fn book(book: &core::Audiobook) -> Audiobook {
    Audiobook {
        asin: Asin::from(book.asin.as_str()),
        marketplace: book.marketplace,
        title: book.title.clone(),
        subtitle: book.subtitle.clone(),
        authors: book.authors.clone(),
        narrators: book.narrators.clone(),
        runtime_minutes: book.runtime_minutes.and_then(|m| u32::try_from(m).ok()),
        cover: None,
        cover_url: book.cover_image_url.as_ref().map(ToString::to_string),
        size_bytes: 0,
    }
}

fn app_error(error: &presentation::AppError) -> AppError {
    use presentation::AppError as Source;
    match error {
        Source::Download {
            book_title,
            technical_error,
        } => AppError::Download {
            book_title: book_title.clone(),
            technical: technical_error.clone(),
        },
        Source::Export {
            book_title,
            technical_error,
        } => AppError::Export {
            book_title: book_title.clone(),
            technical: technical_error.clone(),
        },
        Source::Enqueue {
            kind,
            book_count,
            technical_error,
        } => AppError::Enqueue {
            kind: job_kind(*kind),
            book_count: *book_count,
            technical: technical_error.clone(),
        },
        Source::SignInBrowser { technical_error } => AppError::SignInBrowser {
            technical: technical_error.clone(),
        },
        Source::CredentialStorage(failure) => AppError::CredentialStorage {
            operation: match failure.operation {
                core::CredentialOperation::Load => CredentialOperation::Load,
                core::CredentialOperation::Save => CredentialOperation::Save,
                core::CredentialOperation::Delete => CredentialOperation::Delete,
            },
            technical: failure.technical_error.clone(),
        },
    }
}

impl LiveApp {
    pub fn new(handle: AppHandle) -> Self {
        let snapshot = handle.snapshot();
        let mut app = Self {
            handle,
            books_source: Arc::default(),
            snapshot: snapshot.clone(),
            books: Vec::new(),
            positions: HashMap::new(),
            tiles: Vec::new(),
            account: None,
            pending_login: None,
            browser_page: None,
            toasts: Vec::new(),
            toast_ids: HashMap::new(),
            next_toast_id: 1,
            copied_toasts: HashSet::new(),
            messages: Vec::new(),
            notices: Vec::new(),
            others: Vec::new(),
            suspending: None,
            has_suspended: false,
        };
        app.apply(snapshot);
        app
    }

    fn apply(&mut self, snapshot: Arc<AppSnapshot>) {
        let (session, library, export) = (&snapshot.session, &snapshot.library, &snapshot.export);
        if !Arc::ptr_eq(&self.books_source, &library.visible_books) {
            self.books_source = library.visible_books.clone();
            self.books = self.books_source.iter().map(book).collect();
            self.positions = self
                .books
                .iter()
                .enumerate()
                .map(|(index, book)| (book.asin.clone(), index))
                .collect();
        }
        self.tiles = library
            .visible_books
            .iter()
            .zip(&library.statuses)
            .map(|(source, book_status)| BookTileState {
                is_selected: library.selection.contains(&source.asin),
                is_in_library: snapshot.is_in_library(&source.asin),
                marketplace_badge: snapshot.marketplace_badge(source),
                status: status(book_status),
                menu_items: snapshot
                    .menu_items(source)
                    .into_iter()
                    .map(|item| BookMenuItem {
                        action: action(item.action),
                        disabled_reason: (!item.is_enabled)
                            .then(|| item.disabled_reason.unwrap_or_else(|| "Unavailable".into())),
                    })
                    .collect(),
            })
            .collect();
        self.account = session.is_signed_in.then(|| Account {
            customer_name: session.customer_name.clone(),
            marketplace: session.marketplace,
        });
        self.pending_login =
            (session.pending_login.is_some() || session.is_registering).then(|| {
                PendingLogin::new(
                    session.marketplace,
                    session.is_registering,
                    session.pasted_callback_url.clone(),
                    session.error_message.clone(),
                )
            });
        self.browser_page = session.pending_login.as_ref().map(BrowserPage::for_login);
        for notice in &snapshot.notices {
            if !self.toast_ids.contains_key(&notice.id) {
                self.toast_ids.insert(notice.id, self.next_toast_id);
                self.next_toast_id += 1;
            }
        }
        let (ids, copied) = (&self.toast_ids, &self.copied_toasts);
        self.toasts = snapshot
            .notices
            .iter()
            .map(|notice| {
                let id = ids[&notice.id];
                Toast {
                    id,
                    error: app_error(&notice.error),
                    is_copied: copied.contains(&id),
                }
            })
            .collect();
        self.messages = [
            session.error_message.clone(),
            library.error_message.clone(),
            export.error_message.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.notices = [
            export.notice.map(str::to_string),
            library.marketplace_notice.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.others = library
            .other_marketplaces
            .iter()
            .map(|other| (other.marketplace, other.count))
            .collect();
        self.snapshot = snapshot;
    }

    fn source_book(&self, asin: &str) -> Option<&core::Audiobook> {
        let index = *self.positions.get(asin)?;
        self.snapshot.library.visible_books.get(index)
    }
}

impl AppModel for LiveApp {
    fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    fn needs_reauthentication(&self) -> bool {
        self.snapshot.session.needs_reauthentication
    }

    fn marketplace(&self) -> Marketplace {
        self.snapshot.session.marketplace
    }

    fn pending_login(&self) -> Option<&PendingLogin> {
        self.pending_login.as_ref()
    }

    fn is_loading(&self) -> bool {
        self.snapshot.library.is_loading
    }

    fn is_checking_marketplaces(&self) -> bool {
        self.snapshot.library.is_checking_marketplaces
    }

    fn is_library_empty(&self) -> bool {
        self.snapshot.library.book_count == 0
    }

    fn can_select(&self) -> bool {
        self.snapshot.library.can_select
    }

    fn has_restored(&self) -> bool {
        self.snapshot.session.has_restored
    }

    fn visible_books(&self) -> Vec<&Audiobook> {
        self.books.iter().collect()
    }

    fn tile(&self, book: &Audiobook) -> Cow<'_, BookTileState> {
        self.positions
            .get(&book.asin)
            .and_then(|index| self.tiles.get(*index))
            .map_or_else(|| Cow::Owned(BookTileState::new()), Cow::Borrowed)
    }

    fn has_download(&self, book: &Audiobook) -> bool {
        self.snapshot.downloaded_file(&book.asin).is_some()
    }

    fn messages(&self) -> Vec<String> {
        self.messages.clone()
    }

    fn notices(&self) -> Vec<String> {
        self.notices.clone()
    }

    fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    fn selected_count(&self) -> usize {
        self.snapshot.library.selection.len()
    }

    fn other_marketplaces(&self) -> Vec<(Marketplace, usize)> {
        self.others.clone()
    }

    fn selection_offering(&self, action: BookAction) -> usize {
        match action {
            BookAction::Download => self.snapshot.library.selected_for_download,
            BookAction::Export => self.snapshot.library.selected_for_export,
            _ => 0,
        }
    }

    fn can_sign_in(&self) -> bool {
        self.snapshot.session.can_sign_in && self.snapshot.session.pending_login.is_none()
    }

    fn can_change_marketplace(&self) -> bool {
        self.snapshot.session.can_change_marketplace
    }

    fn can_sign_out(&self) -> bool {
        self.snapshot.session.can_sign_out
    }

    fn can_refresh(&self) -> bool {
        self.snapshot.library.can_refresh
    }

    fn can_submit_pasted_url(&self) -> bool {
        self.snapshot.session.can_submit_pasted_url
    }

    fn browser_page(&self) -> Option<BrowserPage> {
        self.browser_page.clone()
    }

    fn login_url(&self) -> Option<String> {
        let login = self.snapshot.session.pending_login.as_ref()?;
        Some(login.login_url.to_string())
    }

    fn has_active_jobs(&self) -> bool {
        self.snapshot.has_active_jobs
    }

    fn has_work_suspend_loses(&self) -> bool {
        self.snapshot.export.has_work_suspend_loses
    }

    fn has_finished_suspending(&self) -> bool {
        self.has_suspended
    }

    fn is_busy(&self) -> bool {
        self.is_loading()
            || self.is_checking_marketplaces()
            || self.snapshot.session.is_registering
            || self.suspending.is_some()
    }

    fn set_marketplace(&mut self, marketplace: Marketplace) {
        self.handle.send(Command::SetMarketplace(marketplace));
    }

    fn sign_in(&mut self) {
        self.handle.send(Command::SignIn);
    }

    fn complete_sign_in(&mut self, url: Option<String>) {
        if let Some(url) = url {
            self.handle.send(Command::CompleteSignIn(url));
        }
    }

    fn set_pasted_url(&mut self, url: String) {
        self.handle.send(Command::SetPastedCallbackUrl(url));
    }

    fn submit_pasted_url(&mut self) {
        self.handle.send(Command::SubmitPastedUrl);
    }

    fn browser_failed(&mut self, message: String) {
        self.handle.send(Command::BrowserFailed(message));
    }

    fn cancel_sign_in(&mut self) {
        self.handle.send(Command::CancelSignIn);
    }

    fn sign_out(&mut self) {
        self.handle.send(Command::SignOut);
    }

    fn refresh(&mut self) {
        self.handle.send(Command::Refresh);
    }

    fn set_search(&mut self, search: String) {
        self.handle.send(Command::SetSearchText(search));
    }

    fn toggle_selection(&mut self, asin: &str) {
        self.handle.send(Command::ToggleSelection(asin.to_string()));
    }

    fn clear_selection(&mut self) {
        self.handle.send(Command::ClearSelection);
    }

    fn perform(&mut self, wanted: BookAction, asin: &str) -> Option<ModelEffect> {
        match wanted {
            BookAction::ShowDownload => {
                let path = self.snapshot.downloaded_file(asin)?;
                return Some(ModelEffect::Reveal(path.to_path_buf()));
            }
            BookAction::ShowExport => {
                let path = self.snapshot.exported_file(asin)?;
                return Some(ModelEffect::Reveal(path.to_path_buf()));
            }
            _ => {}
        }
        let source = self.source_book(asin)?.clone();
        // The menu entry knows which job a Cancel refers to.
        let item = self
            .snapshot
            .menu_items(&source)
            .into_iter()
            .find(|item| action(item.action) == wanted)?;
        self.handle
            .send(Command::Perform(item.action, Box::new(source)));
        None
    }

    fn perform_on_selection(&mut self, wanted: BookAction) {
        let action = match wanted {
            BookAction::Download => core::BookAction::Download,
            BookAction::Export => core::BookAction::Export,
            _ => return,
        };
        self.handle.send(Command::PerformOnSelection(action));
    }

    fn copy_toast(&mut self, id: u64) -> Option<String> {
        let toast = self.toasts.iter_mut().find(|toast| toast.id == id)?;
        toast.is_copied = true;
        self.copied_toasts.insert(id);
        Some(toast.error.technical().to_string())
    }

    fn dismiss_toast(&mut self, id: u64) {
        let notice = self
            .toast_ids
            .iter()
            .find_map(|(notice, value)| (*value == id).then_some(*notice));
        if let Some(notice) = notice {
            self.handle.send(Command::DismissNotice(notice));
        }
    }

    fn begin_suspend(&mut self) {
        if self.suspending.is_some() || self.has_suspended {
            return;
        }
        let (reply, done) = oneshot::channel();
        self.handle.send(Command::Suspend(reply));
        self.suspending = Some(done);
    }

    fn rescan(&mut self) {
        self.handle.send(Command::Rescan);
    }

    fn tick(&mut self, _elapsed_ms: u32) {
        if let Some(done) = &mut self.suspending
            && !matches!(done.try_recv(), Err(oneshot::error::TryRecvError::Empty))
        {
            self.suspending = None;
            self.has_suspended = true;
        }
        let latest = self.handle.snapshot();
        if !Arc::ptr_eq(&latest, &self.snapshot) {
            self.apply(latest);
        }
    }
}
