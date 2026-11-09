use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use loudible_core::text::standard_compare;
use loudible_core::{
    Audiobook, ExporterError, LibrarySelection, LibraryService, Marketplace, MarketplaceScan,
    is_hidden, merge, search_books,
};
use tokio::task::AbortHandle;

use crate::Changes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

/// A marketplace other than the signed-in one that holds titles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarketplaceCount {
    pub marketplace: Marketplace,
    /// Titles shown from this store (not counting ones the home store also has).
    pub count: usize,
}

struct State {
    books: Vec<Audiobook>,
    /// Filled on demand: folding every title for each progress update would be wasted work.
    visible: Option<Arc<Vec<Audiobook>>>,
    load_state: LoadState,
    is_checking_marketplaces: bool,
    home_marketplace: Option<Marketplace>,
    other_marketplaces: Vec<MarketplaceCount>,
    unchecked_marketplaces: Vec<Marketplace>,
    selection: LibrarySelection,
    search_text: String,
    is_signed_in: bool,
    home_books: Vec<Audiobook>,
    foreign_books: HashMap<&'static str, Vec<Audiobook>>,
    remote_books: Vec<Audiobook>,
    remote_asins: Arc<HashSet<String>>,
    local_books: Vec<Audiobook>,
    marketplace_task: Option<AbortHandle>,
    /// Changes with every refresh and reset, so a late answer can tell it is stale.
    generation: u64,
}

impl State {
    /// Signed out, the grid stays empty; while re-authenticating the last library stays visible.
    fn rebuild_books(&mut self) {
        let shows_local = self.is_signed_in || !self.remote_books.is_empty();
        let mut extra: Vec<Audiobook> = if shows_local {
            self.local_books
                .iter()
                .filter(|book| !self.remote_asins.contains(&book.asin))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        extra.sort_by(|a, b| standard_compare(&a.title, &b.title));
        let mut merged = self.remote_books.clone();
        merged.extend(extra);
        merged.retain(|book| !is_hidden(book));
        if merged == self.books {
            return;
        }
        self.books = merged;
        self.visible = None;
        if !self.is_checking_marketplaces {
            self.selection.reconcile(&self.books);
        }
    }

    fn rebuild_remote(&mut self) {
        let mut merged = self.home_books.clone();
        let mut counts = Vec::new();
        for marketplace in Marketplace::ALL {
            if Some(marketplace) == self.home_marketplace {
                continue;
            }
            let Some(books) = self.foreign_books.get(marketplace.id) else {
                continue;
            };
            let before = merged.len();
            merged = merge(merged, books.clone());
            if merged.len() > before {
                counts.push(MarketplaceCount {
                    marketplace,
                    count: merged.len() - before,
                });
            }
        }
        self.remote_asins = Arc::new(merged.iter().map(|book| book.asin.clone()).collect());
        self.remote_books = merged;
        self.other_marketplaces = counts;
        self.rebuild_books();
    }

    fn cancel_marketplace_task(&mut self) {
        if let Some(task) = self.marketplace_task.take() {
            task.abort();
        }
        self.is_checking_marketplaces = false;
    }
}

struct Inner {
    service: LibraryService,
    state: RefCell<State>,
    changes: Changes,
}

/// The purchased library with search and multi-selection. Downloaded books that are no longer
/// in the library (returned, expired, filtered out) are appended from their stored details.
///
/// A refresh loads the signed-in marketplace first and shows it, then checks every other
/// marketplace and merges the titles found there.
#[derive(Clone)]
pub struct LibraryViewModel(Rc<Inner>);

impl LibraryViewModel {
    pub fn new(service: LibraryService, changes: Changes) -> Self {
        Self(Rc::new(Inner {
            service,
            changes,
            state: RefCell::new(State {
                books: Vec::new(),
                visible: None,
                load_state: LoadState::Idle,
                is_checking_marketplaces: false,
                home_marketplace: None,
                other_marketplaces: Vec::new(),
                unchecked_marketplaces: Vec::new(),
                selection: LibrarySelection::default(),
                search_text: String::new(),
                is_signed_in: false,
                home_books: Vec::new(),
                foreign_books: HashMap::new(),
                remote_books: Vec::new(),
                remote_asins: Arc::default(),
                local_books: Vec::new(),
                marketplace_task: None,
                generation: 0,
            }),
        }))
    }

    fn read<R>(&self, body: impl FnOnce(&State) -> R) -> R {
        body(&self.0.state.borrow())
    }

    fn update<R>(&self, body: impl FnOnce(&mut State) -> R) -> R {
        let result = body(&mut self.0.state.borrow_mut());
        self.0.changes.mark();
        result
    }

    // MARK: Derived state

    pub fn books(&self) -> Vec<Audiobook> {
        self.read(|state| state.books.clone())
    }

    pub fn load_state(&self) -> LoadState {
        self.read(|state| state.load_state.clone())
    }

    pub fn is_checking_marketplaces(&self) -> bool {
        self.read(|state| state.is_checking_marketplaces)
    }

    pub fn home_marketplace(&self) -> Option<Marketplace> {
        self.read(|state| state.home_marketplace)
    }

    pub fn other_marketplaces(&self) -> Vec<MarketplaceCount> {
        self.read(|state| state.other_marketplaces.clone())
    }

    pub fn unchecked_marketplaces(&self) -> Vec<Marketplace> {
        self.read(|state| state.unchecked_marketplaces.clone())
    }

    pub fn selection(&self) -> LibrarySelection {
        self.read(|state| state.selection.clone())
    }

    pub fn search_text(&self) -> String {
        self.read(|state| state.search_text.clone())
    }

    pub fn set_search_text(&self, text: impl Into<String>) {
        let text = text.into();
        self.update(|state| {
            state.search_text = text;
            state.visible = None;
        });
    }

    pub fn is_signed_in(&self) -> bool {
        self.read(|state| state.is_signed_in)
    }

    /// Maintained by the owner; refresh and selection need a signed-in account.
    pub fn set_signed_in(&self, is_signed_in: bool) {
        self.update(|state| {
            state.is_signed_in = is_signed_in;
            state.rebuild_books();
        });
    }

    pub fn is_loading(&self) -> bool {
        self.read(|state| state.load_state == LoadState::Loading)
    }

    pub fn error_message(&self) -> Option<String> {
        self.read(|state| match &state.load_state {
            LoadState::Failed(message) => Some(message.clone()),
            _ => None,
        })
    }

    pub fn visible_books(&self) -> Arc<Vec<Audiobook>> {
        let mut state = self.0.state.borrow_mut();
        if let Some(visible) = &state.visible {
            return visible.clone();
        }
        let visible = Arc::new(search_books(&state.books, &state.search_text));
        state.visible = Some(visible.clone());
        visible
    }

    pub fn selected_books(&self) -> Vec<Audiobook> {
        self.read(|state| state.selection.selected_books(&state.books))
    }

    pub fn can_refresh(&self) -> bool {
        self.is_signed_in() && !self.is_loading() && !self.is_checking_marketplaces()
    }

    pub fn can_select(&self) -> bool {
        self.read(|state| state.is_signed_in && !state.books.is_empty())
    }

    pub fn summary(&self) -> String {
        let visible = self.visible_books().len();
        self.read(|state| {
            let total = state.books.len();
            let mut base = if visible == total {
                format!("{total} books")
            } else {
                format!("{visible} of {total} books")
            };
            if !state.other_marketplaces.is_empty() {
                let names: Vec<&str> = state
                    .other_marketplaces
                    .iter()
                    .map(|other| other.marketplace.name)
                    .collect();
                base += &format!(" · also in {}", names.join(", "));
            }
            if state.selection.is_empty() {
                base
            } else {
                format!("{base}, {} selected", state.selection.len())
            }
        })
    }

    /// Not an error: the library is complete apart from these stores.
    pub fn marketplace_notice(&self) -> Option<String> {
        self.read(|state| {
            if state.unchecked_marketplaces.is_empty() {
                return None;
            }
            let names: Vec<&str> = state
                .unchecked_marketplaces
                .iter()
                .map(|marketplace| marketplace.name)
                .collect();
            let noun = if names.len() == 1 {
                "marketplace"
            } else {
                "marketplaces"
            };
            Some(format!(
                "Couldn’t check the Audible {} {noun} for more titles.",
                names.join(", ")
            ))
        })
    }

    /// A short store label for books from a marketplace other than the signed-in one.
    pub fn marketplace_badge(&self, book: &Audiobook) -> Option<String> {
        marketplace_badge(self.home_marketplace(), book)
    }

    /// Whether the book was in the last refreshed library (false for downloaded books that left it).
    pub fn is_in_library(&self, asin: &str) -> bool {
        self.read(|state| state.remote_asins.contains(asin))
    }

    pub(crate) fn remote_asins(&self) -> Arc<HashSet<String>> {
        self.read(|state| state.remote_asins.clone())
    }

    pub fn is_selected(&self, asin: &str) -> bool {
        self.read(|state| state.selection.contains(asin))
    }

    // MARK: Actions

    /// Reloads the library: the home marketplace, then the others. If the home store fails, the
    /// previous rows stay visible alongside the error and the others are not checked.
    pub async fn refresh(&self) {
        if !self.is_signed_in() || self.is_loading() {
            return;
        }
        let current = self.update(|state| {
            state.cancel_marketplace_task();
            state.load_state = LoadState::Loading;
            state.generation += 1;
            state.generation
        });
        let result: Result<(Marketplace, Vec<Audiobook>), ExporterError> =
            self.0.service.fetch_home_library().await;
        // Reset (signed out) meanwhile: the answer belongs to the old account.
        if self.read(|state| state.generation != current) {
            return;
        }
        let home = match result {
            Ok((home, books)) => {
                self.update(|state| state.home_books = books);
                home
            }
            Err(error) => {
                self.update(|state| state.load_state = LoadState::Failed(error.to_string()));
                return;
            }
        };
        let model = self.clone();
        let task =
            tokio::task::spawn_local(async move { model.check_other_marketplaces(home).await });
        self.update(|state| {
            state.home_marketplace = Some(home);
            state.foreign_books.remove(home.id);
            // Selected titles from other stores are still to come: reconcile once they are in.
            state.is_checking_marketplaces = true;
            state.rebuild_remote();
            state.load_state = LoadState::Loaded;
            state.marketplace_task = Some(task.abort_handle());
        });
        let _ = task.await;
    }

    /// Aborted by a reset or a newer refresh, which then owns the state.
    async fn check_other_marketplaces(&self, home: Marketplace) {
        let mut unchecked = Vec::new();
        let mut scans = self.0.service.fetch_other_marketplaces(home);
        while let Some(scan) = scans.recv().await {
            self.update(|state| {
                match scan {
                    MarketplaceScan::Titles(marketplace, books) => {
                        state.foreign_books.insert(marketplace.id, books);
                    }
                    MarketplaceScan::Empty(marketplace) => {
                        state.foreign_books.remove(marketplace.id);
                    }
                    MarketplaceScan::Failed(marketplace, _) => unchecked.push(marketplace),
                }
                state.rebuild_remote();
            });
        }
        self.update(|state| {
            state.unchecked_marketplaces = unchecked;
            state.is_checking_marketplaces = false;
            state.marketplace_task = None;
            let State {
                selection, books, ..
            } = state;
            selection.reconcile(books);
        });
    }

    /// Replaces the downloaded books shown alongside the library.
    pub fn apply_local(&self, books: Vec<Audiobook>) {
        self.update(|state| {
            if books != state.local_books {
                state.local_books = books;
                state.rebuild_books();
            }
        });
    }

    pub fn toggle_selection(&self, asin: &str) {
        if !self.can_select() {
            return;
        }
        self.update(|state| {
            if state.books.iter().any(|book| book.asin == asin) {
                state.selection.toggle(asin);
            }
        });
    }

    /// Replaces the selection, e.g. from a table's selection binding. Unknown ASINs are ignored.
    pub fn set_selection<I: IntoIterator<Item = S>, S: Into<String>>(&self, asins: I) {
        if !self.can_select() {
            return;
        }
        let selection = LibrarySelection::new(asins);
        self.update(|state| {
            state.selection = selection;
            let State {
                selection, books, ..
            } = state;
            selection.reconcile(books);
        });
    }

    pub fn select_all_visible(&self) {
        if !self.can_select() {
            return;
        }
        let visible = self.visible_books();
        self.update(|state| state.selection.select(&visible));
    }

    pub fn deselect_all_visible(&self) {
        let visible = self.visible_books();
        self.update(|state| state.selection.deselect(&visible));
    }

    pub fn clear_selection(&self) {
        self.update(|state| state.selection.clear());
    }

    /// Forgets the library, e.g. after signing out.
    pub fn reset(&self) {
        self.update(|state| {
            state.generation += 1;
            state.cancel_marketplace_task();
            state.home_marketplace = None;
            state.home_books.clear();
            state.foreign_books.clear();
            state.other_marketplaces.clear();
            state.unchecked_marketplaces.clear();
            state.remote_books.clear();
            state.remote_asins = Arc::default();
            state.local_books.clear();
            state.books.clear();
            state.visible = None;
            state.selection.clear();
            state.search_text.clear();
            state.load_state = LoadState::Idle;
        });
    }
}

pub(crate) fn marketplace_badge(home: Option<Marketplace>, book: &Audiobook) -> Option<String> {
    home.filter(|home| book.marketplace != *home)
        .map(|_| book.marketplace.id.to_uppercase())
}
