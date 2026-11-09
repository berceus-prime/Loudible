use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::text::{contains_folded, split_whitespace};
use crate::{
    AudibleApi, Audiobook, AuthenticationCoordinator, ExporterError, LibraryEntry, LibraryPage,
    Marketplace, RelationshipDirection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exclusion {
    PlusCatalog,
    Podcast,
    Container,
}

/// Title fragments whose books are never shown, matched case- and diacritic-insensitively.
pub const HIDDEN_TITLE_FRAGMENTS: [&str; 1] = ["Morning"];

/// Which library items count as purchased audiobooks: Audible Plus catalog titles, podcasts, and
/// series/podcast containers are excluded.
pub fn exclusion(entry: &LibraryEntry) -> Option<Exclusion> {
    let delivery_type = entry
        .content_delivery_type
        .as_deref()
        .map(str::to_lowercase);
    let delivery_type = delivery_type.as_deref();
    let has = |direction, types: &[&str]| {
        entry.relationships.iter().any(|relationship| {
            relationship.direction == Some(direction)
                && relationship
                    .relationship_type
                    .as_deref()
                    .is_some_and(|kind| types.contains(&kind))
        })
    };
    // A parent of episodes/seasons is a container, not a listenable title.
    if has(
        RelationshipDirection::Child,
        &["episode", "season", "series"],
    ) || matches!(delivery_type, Some("podcastparent" | "podcastseason"))
    {
        return Some(Exclusion::Container);
    }
    if delivery_type == Some("podcastepisode")
        || entry
            .content_type
            .as_deref()
            .map(str::to_lowercase)
            .as_deref()
            == Some("podcast")
        || has(RelationshipDirection::Parent, &["episode"])
    {
        return Some(Exclusion::Podcast);
    }
    if entry.is_ayce == Some(true) {
        return Some(Exclusion::PlusCatalog);
    }
    None
}

/// Whether the book is left out of the library, remote or downloaded.
pub fn is_hidden(book: &Audiobook) -> bool {
    HIDDEN_TITLE_FRAGMENTS
        .iter()
        .any(|fragment| contains_folded(&book.title, fragment))
}

/// `existing` followed by the books of `new` whose ASIN it does not have yet, so the home
/// store's copy of a title wins over another store's.
pub fn merge(existing: Vec<Audiobook>, new: Vec<Audiobook>) -> Vec<Audiobook> {
    let mut seen: HashSet<String> = existing.iter().map(|book| book.asin.clone()).collect();
    let mut merged = existing;
    merged.extend(
        new.into_iter()
            .filter(|book| seen.insert(book.asin.clone())),
    );
    merged
}

/// Owned audiobooks in library order, without duplicate ASINs (first occurrence wins).
pub fn purchased_books(entries: Vec<LibraryEntry>) -> Vec<Audiobook> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|entry| exclusion(entry).is_none() && seen.insert(entry.book.asin.clone()))
        .map(|entry| entry.book)
        .collect()
}

/// Case- and diacritic-insensitive search. Every whitespace-separated term must match
/// the title, subtitle, an author, a narrator, the series, or the ASIN.
pub fn search_books(books: &[Audiobook], query: &str) -> Vec<Audiobook> {
    let terms = split_whitespace(query);
    books
        .iter()
        .filter(|book| {
            let fields: Vec<&str> = [
                Some(book.title.as_str()),
                book.subtitle.as_deref(),
                book.series_title.as_deref(),
                Some(book.asin.as_str()),
            ]
            .into_iter()
            .flatten()
            .chain(book.authors.iter().map(String::as_str))
            .chain(book.narrators.iter().map(String::as_str))
            .collect();
            terms
                .iter()
                .all(|term| fields.iter().any(|field| contains_folded(field, term)))
        })
        .cloned()
        .collect()
}

/// The set of selected ASINs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibrarySelection {
    asins: HashSet<String>,
}

impl LibrarySelection {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(asins: I) -> Self {
        Self {
            asins: asins.into_iter().map(Into::into).collect(),
        }
    }

    pub fn asins(&self) -> &HashSet<String> {
        &self.asins
    }

    pub fn is_empty(&self) -> bool {
        self.asins.is_empty()
    }

    pub fn len(&self) -> usize {
        self.asins.len()
    }

    pub fn contains(&self, asin: &str) -> bool {
        self.asins.contains(asin)
    }

    pub fn toggle(&mut self, asin: &str) {
        if !self.asins.remove(asin) {
            self.asins.insert(asin.to_string());
        }
    }

    pub fn set(&mut self, asin: &str, selected: bool) {
        if selected {
            self.asins.insert(asin.to_string());
        } else {
            self.asins.remove(asin);
        }
    }

    pub fn select(&mut self, books: &[Audiobook]) {
        self.asins
            .extend(books.iter().map(|book| book.asin.clone()));
    }

    pub fn deselect(&mut self, books: &[Audiobook]) {
        for book in books {
            self.asins.remove(&book.asin);
        }
    }

    pub fn clear(&mut self) {
        self.asins.clear();
    }

    /// Drops selected ASINs that are no longer in the library.
    pub fn reconcile(&mut self, books: &[Audiobook]) {
        let known: HashSet<&str> = books.iter().map(|book| book.asin.as_str()).collect();
        self.asins.retain(|asin| known.contains(asin.as_str()));
    }

    /// Selected books in library order.
    pub fn selected_books(&self, books: &[Audiobook]) -> Vec<Audiobook> {
        books
            .iter()
            .filter(|book| self.asins.contains(&book.asin))
            .cloned()
            .collect()
    }
}

/// The outcome of loading one marketplace other than the signed-in one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketplaceScan {
    /// The store holds purchased titles.
    Titles(Marketplace, Vec<Audiobook>),
    /// The store holds nothing that counts as purchased.
    Empty(Marketplace),
    /// The store could not be loaded. Affects that store only, never the session.
    Failed(Marketplace, ExporterError),
}

impl MarketplaceScan {
    pub fn marketplace(&self) -> Marketplace {
        match self {
            Self::Titles(marketplace, _)
            | Self::Empty(marketplace)
            | Self::Failed(marketplace, _) => *marketplace,
        }
    }
}

/// Fetches every library page and applies the ownership rules.
///
/// Audible keeps one library per marketplace: a title bought while the Amazon account was set to
/// another country lives only in that country's library. The signed-in (home) store is loaded
/// first; the others can then be checked one by one with the same device registration.
#[derive(Clone)]
pub struct LibraryService {
    api: Arc<dyn AudibleApi>,
    auth: AuthenticationCoordinator,
    page_size: usize,
}

impl LibraryService {
    pub const DEFAULT_PAGE_SIZE: usize = 1000;
    /// Hard stop in case a server keeps returning full pages.
    const MAXIMUM_PAGES: usize = 1000;

    pub fn new(
        api: Arc<dyn AudibleApi>,
        auth: AuthenticationCoordinator,
        page_size: usize,
    ) -> Self {
        Self {
            api,
            auth,
            page_size,
        }
    }

    /// The signed-in marketplace and its purchased books.
    pub async fn fetch_home_library(&self) -> Result<(Marketplace, Vec<Audiobook>), ExporterError> {
        let home = self.auth.valid_session().await?.marketplace;
        Ok((home, self.fetch_purchased_books(home).await?))
    }

    /// The purchased books in `marketplace`. A rejected token is refreshed and retried, and a
    /// second rejection requires signing in again, so use this for the home store only.
    pub async fn fetch_purchased_books(
        &self,
        marketplace: Marketplace,
    ) -> Result<Vec<Audiobook>, ExporterError> {
        self.fetch_pages(|number| async move {
            self.auth
                .with_valid_session(|session| async move {
                    self.api
                        .library_page(number, self.page_size, marketplace, &session)
                        .await
                })
                .await
        })
        .await
    }

    /// Loads every marketplace except `home`, one after another in `Marketplace::ALL` order.
    ///
    /// A store that rejects the request yields `Failed` for that store only: unlike the home
    /// store, a rejection here must never end the session or delete the stored credentials.
    /// Stops when the receiver is dropped.
    pub fn fetch_other_marketplaces(
        &self,
        home: Marketplace,
    ) -> mpsc::UnboundedReceiver<MarketplaceScan> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let service = self.clone();
        tokio::spawn(async move {
            for marketplace in Marketplace::ALL.into_iter().filter(|m| *m != home) {
                tokio::select! {
                    _ = sender.closed() => break,
                    scan = service.scan(marketplace) => {
                        if sender.send(scan).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        receiver
    }

    async fn scan(&self, marketplace: Marketplace) -> MarketplaceScan {
        let result = self
            .fetch_pages(|number| async move {
                // Refreshes (shared with any other caller) when the token is about to expire.
                let session = self.auth.valid_session().await?;
                self.api
                    .library_page(number, self.page_size, marketplace, &session)
                    .await
            })
            .await;
        match result {
            Ok(books) if books.is_empty() => MarketplaceScan::Empty(marketplace),
            Ok(books) => MarketplaceScan::Titles(marketplace, books),
            Err(error) => MarketplaceScan::Failed(marketplace, error),
        }
    }

    async fn fetch_pages<F, Fut>(&self, page: F) -> Result<Vec<Audiobook>, ExporterError>
    where
        F: Fn(usize) -> Fut,
        Fut: Future<Output = Result<LibraryPage, ExporterError>>,
    {
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        let mut received = 0;
        for number in 1..=Self::MAXIMUM_PAGES {
            let page = page(number).await?;
            let count = page.entries.len();
            received += count;
            let before = entries.len();
            entries.extend(
                page.entries
                    .into_iter()
                    .filter(|entry| seen.insert(entry.book.asin.clone())),
            );
            let is_last_page = count < self.page_size
                || page.total_count.is_some_and(|total| received >= total)
                // A page with nothing new means the server is repeating itself.
                || entries.len() == before;
            if is_last_page {
                break;
            }
        }
        Ok(purchased_books(entries))
    }
}
