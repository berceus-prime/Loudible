//! The in-memory app behind the interactive mock: sign-in, library loading, search, selection,
//! and a one-at-a-time download/export queue with fake progress. Time only moves through
//! [`MockApp::tick`], so the shell drives it from a timer and tests drive it by hand.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use super::app_error::AppError;
use super::app_model::{AppModel, ModelEffect};
use super::book::{
    Asin, Audiobook, BookAction, BookMenuItem, BookStatus, BookTileState, JobKind, Marketplace,
    menu_items,
};
use super::preview_data::{self, Account};

const REGISTER_MS: u32 = 900;
const LOAD_MS: u32 = 900;
const CHECK_MARKETPLACES_MS: u32 = 700;
const DOWNLOAD_MS: f32 = 4_000.0;
const EXPORT_MS: f32 = 2_500.0;

/// The first download of this book drops its connection part-way, keeping what it received.
const FLAKY_DOWNLOAD: &str = "PREVIEW003";
/// The first export of this book fails to write its output file.
const FAILING_EXPORT: &str = "PREVIEW009";
/// Downloading this book is always refused, like a title without purchase rights.
const DENIED_DOWNLOAD: &str = "PREVIEW008";

#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    SignedOut { needs_reauthentication: bool },
    SignedIn(Account),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingLogin {
    pub marketplace: Marketplace,
    pub is_registering: bool,
    pub pasted_url: String,
    pub error: Option<String>,
    elapsed_ms: u32,
}

impl PendingLogin {
    pub fn new(
        marketplace: Marketplace,
        is_registering: bool,
        pasted_url: String,
        error: Option<String>,
    ) -> Self {
        Self {
            marketplace,
            is_registering,
            pasted_url,
            error,
            elapsed_ms: 0,
        }
    }

    /// The browser fallback only accepts a pasted Amazon landing address.
    pub fn can_submit_pasted_url(&self) -> bool {
        !self.is_registering && self.pasted_url.contains("/ap/maplanding")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LibraryLoad {
    Idle,
    Loading { elapsed_ms: u32 },
    CheckingMarketplaces { elapsed_ms: u32 },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Job {
    asin: Asin,
    kind: JobKind,
    progress: f32,
}

/// A toast waiting to be dismissed.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub error: AppError,
    pub is_copied: bool,
}

/// Options for starting the mock, from the command line.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockOptions {
    pub signed_in: bool,
    pub fail_load: bool,
}

#[derive(Debug)]
pub struct MockApp {
    session: Session,
    /// The marketplace picked for the next sign-in.
    pub marketplace: Marketplace,
    pending_login: Option<PendingLogin>,
    load: LibraryLoad,
    fail_load: bool,
    books: Vec<Audiobook>,
    search: String,
    selection: BTreeSet<Asin>,
    queue: Vec<Job>,
    downloaded: BTreeSet<Asin>,
    exported: BTreeSet<Asin>,
    failures: BTreeMap<Asin, (JobKind, String)>,
    failed_once: BTreeSet<(Asin, JobKind)>,
    toasts: Vec<Toast>,
    next_toast_id: u64,
    notice: Option<String>,
}

impl MockApp {
    pub fn new(options: MockOptions) -> Self {
        let mut app = Self {
            session: Session::SignedOut {
                needs_reauthentication: false,
            },
            marketplace: Marketplace::US,
            pending_login: None,
            load: LibraryLoad::Idle,
            fail_load: options.fail_load,
            books: vec![],
            search: String::new(),
            selection: BTreeSet::new(),
            queue: vec![],
            downloaded: BTreeSet::new(),
            exported: BTreeSet::new(),
            failures: BTreeMap::new(),
            failed_once: BTreeSet::new(),
            toasts: vec![],
            next_toast_id: 1,
            notice: None,
        };
        if options.signed_in {
            app.session = Session::SignedIn(preview_data::account());
            app.refresh();
        }
        app
    }

    // MARK: Reading

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn account(&self) -> Option<&Account> {
        match &self.session {
            Session::SignedIn(account) => Some(account),
            Session::SignedOut { .. } => None,
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.account().is_some()
    }

    pub fn needs_reauthentication(&self) -> bool {
        matches!(
            self.session,
            Session::SignedOut {
                needs_reauthentication: true
            }
        )
    }

    pub fn pending_login(&self) -> Option<&PendingLogin> {
        self.pending_login.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.load, LibraryLoad::Loading { .. })
    }

    pub fn is_checking_marketplaces(&self) -> bool {
        matches!(self.load, LibraryLoad::CheckingMarketplaces { .. })
    }

    pub fn is_library_empty(&self) -> bool {
        self.books.is_empty()
    }

    /// Every book in the library, whatever the search.
    pub fn book_count(&self) -> usize {
        self.books.len()
    }

    /// Signed out with nothing to show: the window shows its sign-in prompt.
    pub fn is_signed_out_and_empty(&self) -> bool {
        !self.is_signed_in() && self.books.is_empty()
    }

    pub fn can_select(&self) -> bool {
        self.is_signed_in() && !self.books.is_empty()
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    /// The books matching the search as the library lists them: the signed-in store's first,
    /// then the other stores in `Marketplace::ALL` order, the order they are scanned in.
    pub fn visible_books(&self) -> Vec<&Audiobook> {
        let home = self.account().map(|account| account.marketplace);
        let mut books: Vec<&Audiobook> = self
            .books
            .iter()
            .filter(|book| book.matches(&self.search))
            .collect();
        books.sort_by_key(|book| {
            let store = Marketplace::ALL.iter().position(|m| *m == book.marketplace);
            (Some(book.marketplace) != home, store)
        });
        books
    }

    /// Error messages for the bar above the grid.
    pub fn messages(&self) -> Vec<String> {
        match &self.load {
            LibraryLoad::Failed(message) => vec![message.clone()],
            _ => vec![],
        }
    }

    pub fn notices(&self) -> Vec<String> {
        self.notice.iter().cloned().collect()
    }

    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// Titles in other stores than the signed-in one, with their count.
    pub fn other_marketplaces(&self) -> Vec<(Marketplace, usize)> {
        let Some(home) = self.account().map(|account| account.marketplace) else {
            return vec![];
        };
        Marketplace::ALL
            .into_iter()
            .filter(|marketplace| *marketplace != home)
            .map(|marketplace| {
                (
                    marketplace,
                    self.books
                        .iter()
                        .filter(|b| b.marketplace == marketplace)
                        .count(),
                )
            })
            .filter(|(_, count)| *count > 0)
            .collect()
    }

    pub fn status(&self, book: &Audiobook) -> BookStatus {
        let jobs: Vec<&Job> = self
            .queue
            .iter()
            .filter(|job| job.asin == book.asin)
            .collect();
        if let Some(job) = jobs
            .first()
            .filter(|_| self.queue.first().map(|j| &j.asin) == Some(&book.asin))
        {
            return BookStatus::Running {
                kind: job.kind,
                progress: job.progress,
                transferred: (job.kind == JobKind::Download)
                    .then_some(book.transferred(job.progress)),
            };
        }
        if let Some(job) = jobs.first() {
            return BookStatus::Queued(job.kind);
        }
        if let Some((kind, message)) = self.failures.get(&book.asin) {
            return BookStatus::Failed {
                kind: *kind,
                message: message.clone(),
            };
        }
        if self.exported.contains(&book.asin) {
            return BookStatus::Exported;
        }
        if self.downloaded.contains(&book.asin) {
            return BookStatus::Downloaded;
        }
        BookStatus::None
    }

    pub fn tile(&self, book: &Audiobook) -> BookTileState {
        let status = self.status(book);
        let home = self.account().map(|account| account.marketplace);
        BookTileState {
            is_selected: self.selection.contains(&book.asin),
            is_in_library: true,
            marketplace_badge: (home.is_some_and(|home| home != book.marketplace))
                .then(|| book.marketplace.id.to_uppercase()),
            menu_items: if self.is_signed_in() {
                menu_items(&status, true)
            } else {
                vec![]
            },
            status,
        }
    }

    /// How many selected books offer `action`.
    pub fn selection_offering(&self, action: BookAction) -> usize {
        self.selected_books()
            .filter(|book| {
                self.tile(book)
                    .menu_items
                    .iter()
                    .any(|item: &BookMenuItem| item.is_enabled() && item.action == action)
            })
            .count()
    }

    fn selected_books(&self) -> impl Iterator<Item = &Audiobook> {
        self.books
            .iter()
            .filter(|book| self.selection.contains(&book.asin))
    }

    fn book(&self, asin: &str) -> Option<&Audiobook> {
        self.books.iter().find(|book| &*book.asin == asin)
    }

    // MARK: Session

    pub fn set_marketplace(&mut self, marketplace: Marketplace) {
        self.marketplace = marketplace;
    }

    pub fn sign_in(&mut self) {
        if self.pending_login.is_none() {
            self.pending_login = Some(PendingLogin::new(
                self.marketplace,
                false,
                String::new(),
                None,
            ));
        }
    }

    /// The embedded page "finished": register the device, then sign in.
    pub fn complete_sign_in(&mut self) {
        if let Some(login) = &mut self.pending_login {
            login.is_registering = true;
            login.error = None;
            login.elapsed_ms = 0;
        }
    }

    pub fn set_pasted_url(&mut self, url: String) {
        if let Some(login) = &mut self.pending_login {
            login.pasted_url = url;
        }
    }

    pub fn submit_pasted_url(&mut self) {
        if self
            .pending_login
            .as_ref()
            .is_some_and(PendingLogin::can_submit_pasted_url)
        {
            self.complete_sign_in();
        } else if let Some(login) = &mut self.pending_login {
            login.error = Some(preview_data::LOGIN_EXPIRED.into());
        }
    }

    pub fn browser_failed(&mut self) {
        self.present(preview_data::sign_in_browser());
    }

    pub fn cancel_sign_in(&mut self) {
        self.pending_login = None;
    }

    pub fn sign_out(&mut self) {
        self.queue.clear();
        self.session = Session::SignedOut {
            needs_reauthentication: false,
        };
        self.books.clear();
        self.selection.clear();
        self.search.clear();
        self.load = LibraryLoad::Idle;
        self.notice = None;
    }

    /// Ends the session as if its tokens expired; the library stays visible.
    pub fn expire_session(&mut self) {
        self.session = Session::SignedOut {
            needs_reauthentication: true,
        };
        self.queue.clear();
    }

    // MARK: Library

    pub fn refresh(&mut self) {
        if self.is_signed_in() && !self.is_loading() && !self.is_checking_marketplaces() {
            self.load = LibraryLoad::Loading { elapsed_ms: 0 };
        }
    }

    pub fn set_search(&mut self, search: String) {
        self.search = search;
    }

    pub fn toggle_selection(&mut self, asin: &str) {
        if !self.can_select() {
            return;
        }
        if !self.selection.remove(asin) {
            self.selection.insert(asin.into());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    // MARK: Queue

    pub fn perform(&mut self, action: BookAction, asin: &str) {
        let asin = Asin::from(asin);
        match action {
            BookAction::Download => self.enqueue(asin, JobKind::Download),
            BookAction::Export => {
                if !self.downloaded.contains(&asin) {
                    self.enqueue(asin.clone(), JobKind::Download);
                }
                self.enqueue(asin, JobKind::Export);
            }
            BookAction::Cancel(kind) => {
                if let Some(index) = self
                    .queue
                    .iter()
                    .position(|j| j.asin == asin && j.kind == kind)
                {
                    self.queue.remove(index);
                    // An export waiting on this download cannot run any more.
                    if kind == JobKind::Download && !self.downloaded.contains(&asin) {
                        self.queue.retain(|j| j.asin != asin);
                    }
                }
            }
            BookAction::ShowDownload | BookAction::ShowExport => {
                self.notice = Some("Finder isn’t available in the mock app.".into());
            }
            BookAction::DeleteDownload => {
                self.downloaded.remove(&asin);
                self.failures.remove(&asin);
            }
        }
    }

    /// Download or Export for every selected book that offers it.
    pub fn perform_on_selection(&mut self, action: BookAction) {
        let asins: Vec<Asin> = self
            .selected_books()
            .map(|book| book.asin.clone())
            .collect();
        for asin in asins {
            let Some(book) = self.book(&asin) else {
                continue;
            };
            let items = self.tile(book).menu_items;
            let offered =
                |wanted: BookAction| items.iter().any(|i| i.is_enabled() && i.action == wanted);
            if offered(action) {
                self.perform(action, &asin);
            }
        }
        self.selection.clear();
    }

    fn enqueue(&mut self, asin: Asin, kind: JobKind) {
        if self
            .queue
            .iter()
            .any(|job| job.asin == asin && job.kind == kind)
        {
            return;
        }
        self.failures.remove(&asin);
        let progress = 0.0;
        self.queue.push(Job {
            asin,
            kind,
            progress,
        });
    }

    // MARK: Toasts

    pub fn present(&mut self, error: AppError) {
        self.toasts.push(Toast {
            id: self.next_toast_id,
            error,
            is_copied: false,
        });
        self.next_toast_id += 1;
    }

    /// Marks the toast copied and returns its technical details for the clipboard.
    pub fn copy_toast(&mut self, id: u64) -> Option<String> {
        let toast = self.toasts.iter_mut().find(|toast| toast.id == id)?;
        toast.is_copied = true;
        Some(toast.error.technical().to_string())
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|toast| toast.id != id);
    }

    // MARK: Time

    /// Whether anything is waiting on time, so the shell knows to keep ticking.
    pub fn is_busy(&self) -> bool {
        self.pending_login
            .as_ref()
            .is_some_and(|login| login.is_registering)
            || self.is_loading()
            || self.is_checking_marketplaces()
            || !self.queue.is_empty()
    }

    pub fn tick(&mut self, elapsed_ms: u32) {
        // The library first: a load started by signing in begins with the next tick.
        self.tick_library(elapsed_ms);
        self.tick_login(elapsed_ms);
        self.tick_queue(elapsed_ms);
    }

    fn tick_login(&mut self, elapsed_ms: u32) {
        let Some(login) = &mut self.pending_login else {
            return;
        };
        if !login.is_registering {
            return;
        }
        login.elapsed_ms += elapsed_ms;
        if login.elapsed_ms >= REGISTER_MS {
            let marketplace = login.marketplace;
            self.pending_login = None;
            self.session = Session::SignedIn(Account {
                marketplace,
                ..preview_data::account()
            });
            self.load = LibraryLoad::Loading { elapsed_ms: 0 };
        }
    }

    fn tick_library(&mut self, elapsed_ms: u32) {
        match &mut self.load {
            LibraryLoad::Loading { elapsed_ms: total } => {
                *total += elapsed_ms;
                if *total >= LOAD_MS {
                    if self.fail_load {
                        self.load = LibraryLoad::Failed(preview_data::LOAD_FAILED.into());
                    } else {
                        self.books = preview_data::books();
                        self.load = LibraryLoad::CheckingMarketplaces { elapsed_ms: 0 };
                    }
                }
            }
            LibraryLoad::CheckingMarketplaces { elapsed_ms: total } => {
                *total += elapsed_ms;
                if *total >= CHECK_MARKETPLACES_MS {
                    self.load = LibraryLoad::Idle;
                }
            }
            LibraryLoad::Idle | LibraryLoad::Failed(_) => {}
        }
    }

    fn tick_queue(&mut self, elapsed_ms: u32) {
        let Some(job) = self.queue.first_mut() else {
            return;
        };
        let asin = job.asin.clone();
        let kind = job.kind;
        let duration = if kind == JobKind::Download {
            DOWNLOAD_MS
        } else {
            EXPORT_MS
        };
        job.progress = (job.progress + elapsed_ms as f32 / duration).min(1.0);
        let progress = job.progress;
        let title = self
            .book(&asin)
            .map_or_else(String::new, |book| book.title.clone());

        let first_attempt = !self.failed_once.contains(&(asin.clone(), kind));
        let failure = match (&*asin, kind) {
            (DENIED_DOWNLOAD, JobKind::Download) if progress > 0.05 => Some((
                "The download license was denied.".to_string(),
                AppError::Download {
                    book_title: title.clone(),
                    technical: format!(
                        "{title}: The download license was denied: License not granted.\nOwnership / RequesterEligibility: No purchase rights found for this title."
                    ),
                },
            )),
            (FLAKY_DOWNLOAD, JobKind::Download) if first_attempt && progress > 0.4 => Some((
                "The network connection was lost.".to_string(),
                AppError::Download {
                    book_title: title.clone(),
                    technical: format!(
                        "{title}: NSURLErrorDomain Code=-1005: The network connection was lost."
                    ),
                },
            )),
            (FAILING_EXPORT, JobKind::Export) if first_attempt && progress > 0.7 => Some((
                "Could not write output file.".to_string(),
                AppError::Export {
                    book_title: title.clone(),
                    technical:
                        "Conversion failed: Could not write output file (Permission denied).".into(),
                },
            )),
            _ => None,
        };

        if let Some((message, error)) = failure {
            self.queue.remove(0);
            self.failed_once.insert((asin.clone(), kind));
            if kind == JobKind::Download {
                self.queue.retain(|j| j.asin != asin);
            }
            self.failures.insert(asin, (kind, message));
            self.present(error);
        } else if progress >= 1.0 {
            self.queue.remove(0);
            match kind {
                JobKind::Download => {
                    self.downloaded.insert(asin);
                }
                JobKind::Export => {
                    self.exported.insert(asin);
                }
            }
        }
    }
}

impl AppModel for MockApp {
    fn account(&self) -> Option<&Account> {
        Self::account(self)
    }

    fn needs_reauthentication(&self) -> bool {
        Self::needs_reauthentication(self)
    }

    fn marketplace(&self) -> Marketplace {
        self.marketplace
    }

    fn pending_login(&self) -> Option<&PendingLogin> {
        Self::pending_login(self)
    }

    fn is_loading(&self) -> bool {
        Self::is_loading(self)
    }

    fn is_checking_marketplaces(&self) -> bool {
        Self::is_checking_marketplaces(self)
    }

    fn is_library_empty(&self) -> bool {
        Self::is_library_empty(self)
    }

    fn can_select(&self) -> bool {
        Self::can_select(self)
    }

    fn visible_books(&self) -> Vec<&Audiobook> {
        Self::visible_books(self)
    }

    fn tile(&self, book: &Audiobook) -> Cow<'_, BookTileState> {
        Cow::Owned(Self::tile(self, book))
    }

    fn has_download(&self, book: &Audiobook) -> bool {
        self.downloaded.contains(&book.asin)
    }

    fn messages(&self) -> Vec<String> {
        Self::messages(self)
    }

    fn notices(&self) -> Vec<String> {
        Self::notices(self)
    }

    fn toasts(&self) -> &[Toast] {
        Self::toasts(self)
    }

    fn selected_count(&self) -> usize {
        Self::selected_count(self)
    }

    fn other_marketplaces(&self) -> Vec<(Marketplace, usize)> {
        Self::other_marketplaces(self)
    }

    fn selection_offering(&self, action: BookAction) -> usize {
        Self::selection_offering(self, action)
    }

    fn can_sign_in(&self) -> bool {
        self.pending_login.is_none()
    }

    fn can_change_marketplace(&self) -> bool {
        self.pending_login.is_none()
    }

    fn can_sign_out(&self) -> bool {
        Self::is_signed_in(self)
    }

    fn can_refresh(&self) -> bool {
        Self::is_signed_in(self)
    }

    fn can_submit_pasted_url(&self) -> bool {
        self.pending_login
            .as_ref()
            .is_some_and(PendingLogin::can_submit_pasted_url)
    }

    fn can_simulate(&self) -> bool {
        true
    }

    fn is_busy(&self) -> bool {
        Self::is_busy(self)
    }

    fn set_marketplace(&mut self, marketplace: Marketplace) {
        Self::set_marketplace(self, marketplace);
    }

    fn sign_in(&mut self) {
        Self::sign_in(self);
    }

    fn complete_sign_in(&mut self, _url: Option<String>) {
        Self::complete_sign_in(self);
    }

    fn set_pasted_url(&mut self, url: String) {
        Self::set_pasted_url(self, url);
    }

    fn submit_pasted_url(&mut self) {
        Self::submit_pasted_url(self);
    }

    fn browser_failed(&mut self, _message: String) {
        Self::browser_failed(self);
    }

    fn cancel_sign_in(&mut self) {
        Self::cancel_sign_in(self);
    }

    fn sign_out(&mut self) {
        Self::sign_out(self);
    }

    fn refresh(&mut self) {
        Self::refresh(self);
    }

    fn set_search(&mut self, search: String) {
        Self::set_search(self, search);
    }

    fn toggle_selection(&mut self, asin: &str) {
        Self::toggle_selection(self, asin);
    }

    fn clear_selection(&mut self) {
        Self::clear_selection(self);
    }

    fn perform(&mut self, action: BookAction, asin: &str) -> Option<ModelEffect> {
        Self::perform(self, action, asin);
        None
    }

    fn perform_on_selection(&mut self, action: BookAction) {
        Self::perform_on_selection(self, action);
    }

    fn copy_toast(&mut self, id: u64) -> Option<String> {
        Self::copy_toast(self, id)
    }

    fn dismiss_toast(&mut self, id: u64) {
        Self::dismiss_toast(self, id);
    }

    fn tick(&mut self, elapsed_ms: u32) {
        Self::tick(self, elapsed_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in() -> MockApp {
        let mut app = MockApp::new(MockOptions {
            signed_in: true,
            fail_load: false,
        });
        app.tick(LOAD_MS + CHECK_MARKETPLACES_MS);
        app
    }

    fn book(app: &MockApp, asin: &str) -> Audiobook {
        app.book(asin).unwrap().clone()
    }

    #[test]
    fn sign_in_registers_then_loads_the_library() {
        let mut app = MockApp::new(MockOptions::default());
        assert!(app.is_signed_out_and_empty());
        app.sign_in();
        app.complete_sign_in();
        app.tick(REGISTER_MS);
        assert!(app.is_signed_in());
        assert!(app.pending_login().is_none());
        assert!(app.is_loading());
        app.tick(LOAD_MS);
        assert_eq!(app.visible_books().len(), 10);
        assert!(app.is_checking_marketplaces());
        app.tick(CHECK_MARKETPLACES_MS);
        assert!(!app.is_busy());
    }

    #[test]
    fn other_stores_follow_home_in_marketplace_order() {
        let app = signed_in();
        let stores: Vec<Marketplace> = app.visible_books().iter().map(|b| b.marketplace).collect();
        let (home, others) =
            stores.split_at(stores.iter().take_while(|m| **m == Marketplace::US).count());
        assert!(!home.is_empty());
        assert_eq!(others.first(), Some(&Marketplace::UK));
        assert_eq!(others.last(), Some(&Marketplace::JAPAN));
        assert!(!others.contains(&Marketplace::US));
    }

    #[test]
    fn failed_load_shows_the_message() {
        let mut app = MockApp::new(MockOptions {
            signed_in: true,
            fail_load: true,
        });
        app.tick(LOAD_MS);
        assert_eq!(app.messages(), vec![preview_data::LOAD_FAILED.to_string()]);
        assert!(app.is_library_empty());
    }

    #[test]
    fn a_pasted_url_must_be_a_landing_page() {
        let mut app = MockApp::new(MockOptions::default());
        app.sign_in();
        app.set_pasted_url("https://example.invalid".into());
        app.submit_pasted_url();
        assert_eq!(
            app.pending_login().unwrap().error.as_deref(),
            Some(preview_data::LOGIN_EXPIRED)
        );
        app.set_pasted_url("https://www.amazon.com/ap/maplanding?code=x".into());
        app.submit_pasted_url();
        assert!(app.pending_login().unwrap().is_registering);
    }

    #[test]
    fn search_matches_title_author_and_narrator() {
        let mut app = signed_in();
        app.set_search("henrik".into());
        assert_eq!(app.visible_books().len(), 1);
        app.set_search("tom avery".into());
        assert_eq!(&*app.visible_books()[0].asin, "PREVIEW001");
        app.set_search("no such book".into());
        assert!(app.visible_books().is_empty());
        assert!(!app.is_library_empty());
    }

    #[test]
    fn badges_mark_other_marketplaces() {
        let app = signed_in();
        assert_eq!(
            app.tile(&book(&app, "PREVIEW004"))
                .marketplace_badge
                .as_deref(),
            Some("UK")
        );
        assert_eq!(app.tile(&book(&app, "PREVIEW001")).marketplace_badge, None);
        assert_eq!(
            app.other_marketplaces(),
            vec![(Marketplace::UK, 2), (Marketplace::JAPAN, 1)]
        );
    }

    #[test]
    fn download_runs_then_completes() {
        let mut app = signed_in();
        app.perform(BookAction::Download, "PREVIEW002");
        app.perform(BookAction::Download, "PREVIEW001");
        app.tick(1_000);
        let tea = book(&app, "PREVIEW002");
        assert!(matches!(
            app.status(&tea),
            BookStatus::Running {
                kind: JobKind::Download,
                ..
            }
        ));
        assert_eq!(
            app.status(&book(&app, "PREVIEW001")),
            BookStatus::Queued(JobKind::Download)
        );
        app.tick(DOWNLOAD_MS as u32);
        assert_eq!(app.status(&tea), BookStatus::Downloaded);
    }

    #[test]
    fn export_of_a_missing_download_downloads_first() {
        let mut app = signed_in();
        app.perform(BookAction::Export, "PREVIEW001");
        app.tick(DOWNLOAD_MS as u32);
        app.tick(EXPORT_MS as u32);
        assert_eq!(app.status(&book(&app, "PREVIEW001")), BookStatus::Exported);
    }

    #[test]
    fn flaky_download_fails_once_then_restarts() {
        let mut app = signed_in();
        app.perform(BookAction::Download, FLAKY_DOWNLOAD);
        app.tick(DOWNLOAD_MS as u32 / 2);
        let deep = book(&app, FLAKY_DOWNLOAD);
        assert!(matches!(app.status(&deep), BookStatus::Failed { .. }));
        assert_eq!(app.toasts().len(), 1);
        let items = app.tile(&deep).menu_items;
        assert_eq!(items[0].action, BookAction::Download);
        app.perform(BookAction::Download, FLAKY_DOWNLOAD);
        app.tick(DOWNLOAD_MS as u32);
        assert_eq!(app.status(&deep), BookStatus::Downloaded);
    }

    #[test]
    fn cancel_discards_a_partial_download() {
        let mut app = signed_in();
        app.perform(BookAction::Download, "PREVIEW002");
        app.tick(1_000);
        app.perform(BookAction::Cancel(JobKind::Download), "PREVIEW002");
        assert!(matches!(
            app.status(&book(&app, "PREVIEW002")),
            BookStatus::None
        ));
        assert_eq!(app.status(&book(&app, "PREVIEW002")), BookStatus::None);
    }

    #[test]
    fn selection_actions_count_offering_books_and_clear() {
        let mut app = signed_in();
        app.toggle_selection("PREVIEW001");
        app.toggle_selection("PREVIEW005");
        assert_eq!(app.selection_offering(BookAction::Download), 2);
        app.perform_on_selection(BookAction::Download);
        assert_eq!(app.selected_count(), 0);
        assert!(app.is_busy());
    }

    #[test]
    fn toasts_copy_and_dismiss_by_id() {
        let mut app = signed_in();
        app.present(preview_data::download_failed());
        app.present(preview_data::credential_save());
        let first = app.toasts()[0].id;
        assert_eq!(app.copy_toast(first).as_deref(), Some("HTTP 503"));
        assert!(app.toasts()[0].is_copied);
        app.dismiss_toast(first);
        assert_eq!(app.toasts().len(), 1);
    }

    #[test]
    fn sign_out_discards_the_running_download() {
        let mut app = signed_in();
        app.perform(BookAction::Download, "PREVIEW002");
        app.tick(1_000);
        app.sign_out();
        assert!(app.is_signed_out_and_empty());
        app.sign_in();
        app.complete_sign_in();
        app.tick(REGISTER_MS);
        app.tick(LOAD_MS);
        assert_eq!(app.status(&book(&app, "PREVIEW002")), BookStatus::None);
        app.perform(BookAction::Download, "PREVIEW002");
        assert!(matches!(
            app.status(&book(&app, "PREVIEW002")),
            BookStatus::Running { progress: 0.0, .. }
        ));
    }
}
