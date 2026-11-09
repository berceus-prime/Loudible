use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use RelationshipDirection::{Child, Parent};
use loudible_core::*;
use loudible_test_support::*;

fn entry_with(
    delivery: &str,
    content: &str,
    ayce: bool,
    relationships: Vec<ProductRelationship>,
) -> LibraryEntry {
    entry_of(book("B1"), delivery, content, ayce, relationships)
}

fn related(direction: RelationshipDirection, kind: &str) -> Vec<ProductRelationship> {
    vec![ProductRelationship::new(
        Some("X".into()),
        Some(direction),
        Some(kind),
    )]
}

#[test]
fn keeps_owned_books() {
    assert_eq!(exclusion(&entry("B1")), None);
    assert_eq!(
        exclusion(&entry_with(
            "MultiPartBook",
            "Product",
            false,
            related(Child, "component")
        )),
        None
    );
    assert_eq!(
        exclusion(&entry_with(
            "SinglePartBook",
            "Product",
            false,
            related(Parent, "series")
        )),
        None
    );
}

#[test]
fn excludes_plus_catalog_titles() {
    assert_eq!(
        exclusion(&entry_with("SinglePartBook", "Product", true, vec![])),
        Some(Exclusion::PlusCatalog)
    );
}

#[test]
fn excludes_podcasts() {
    let podcast = Some(Exclusion::Podcast);
    assert_eq!(
        exclusion(&entry_with("PodcastEpisode", "Podcast", false, vec![])),
        podcast
    );
    assert_eq!(
        exclusion(&entry_with("SinglePartBook", "Podcast", false, vec![])),
        podcast
    );
    assert_eq!(
        exclusion(&entry_with(
            "SinglePartBook",
            "Product",
            false,
            related(Parent, "Episode")
        )),
        podcast
    );
}

#[test]
fn excludes_containers() {
    let container = Some(Exclusion::Container);
    assert_eq!(
        exclusion(&entry_with("PodcastParent", "Product", false, vec![])),
        container
    );
    assert_eq!(
        exclusion(&entry_with("PodcastSeason", "Product", false, vec![])),
        container
    );
    assert_eq!(
        exclusion(&entry_with(
            "SinglePartBook",
            "Product",
            false,
            related(Child, "episode")
        )),
        container
    );
    assert_eq!(
        exclusion(&entry_with(
            "SinglePartBook",
            "Product",
            false,
            related(Child, "season")
        )),
        container
    );
}

#[test]
fn merge_appends_new_titles_and_keeps_the_first_copy() {
    let home = vec![book_in("B1", "Home", Marketplace::US), book("B2")];
    let other = vec![
        book_in("B3", "x", Marketplace::UK),
        book_in("B1", "Other", Marketplace::UK),
    ];
    let merged = merge(home, other);
    assert_eq!(
        merged.iter().map(|b| b.asin.as_str()).collect::<Vec<_>>(),
        ["B1", "B2", "B3"]
    );
    assert!(merged[0].title == "Home" && merged[0].marketplace == Marketplace::US);
    assert_eq!(merged[2].marketplace, Marketplace::UK);
}

#[test]
fn deduplicates_by_asin_keeping_first() {
    let first = entry_of(
        book_in("B1", "First", Marketplace::US),
        "SinglePartBook",
        "Product",
        false,
        vec![],
    );
    let second = entry_of(
        book_in("B1", "Second", Marketplace::US),
        "SinglePartBook",
        "Product",
        false,
        vec![],
    );
    let books = purchased_books(vec![first, entry("B2"), second]);
    assert_eq!(asins(&books), ["B1", "B2"]);
    assert_eq!(books[0].title, "First");
}

fn asins(books: &[Audiobook]) -> Vec<&str> {
    books.iter().map(|book| book.asin.as_str()).collect()
}

struct Fixture {
    api: Arc<MockAudibleApi>,
    store: Arc<InMemoryCredentialStore>,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            api: Arc::default(),
            store: Arc::new(InMemoryCredentialStore::with(session())),
            clock: Arc::default(),
        }
    }

    fn make(&self, page_size: usize) -> (LibraryService, AuthenticationCoordinator) {
        let auth = AuthenticationCoordinator::new(
            self.api.clone(),
            self.store.clone(),
            self.clock.clone(),
        );
        auth.restore().unwrap();
        (
            LibraryService::new(self.api.clone(), auth.clone(), page_size),
            auth,
        )
    }

    fn script(&self, pages: Vec<Vec<LibraryEntry>>, total_count: Option<usize>) {
        self.api.on_library(move |request| {
            let entries = pages.get(request.number - 1).cloned().unwrap_or_default();
            async move {
                Ok(LibraryPage {
                    entries,
                    total_count,
                })
            }
        });
    }

    async fn fetch(&self, page_size: usize) -> Result<Vec<Audiobook>, ExporterError> {
        self.make(page_size)
            .0
            .fetch_purchased_books(Marketplace::US)
            .await
    }

    fn pages(&self) -> Vec<usize> {
        self.api.requested_pages.current()
    }

    /// Each store's ASINs; stores not listed are empty.
    fn script_stores(&self, libraries: Vec<(Marketplace, Vec<&'static str>)>) {
        let libraries: HashMap<Marketplace, Vec<&'static str>> = libraries.into_iter().collect();
        self.api.on_library(move |request| {
            let entries = libraries
                .get(&request.marketplace)
                .map(|asins| {
                    asins
                        .iter()
                        .map(|asin| entry_in(asin, request.marketplace))
                        .collect()
                })
                .unwrap_or_default();
            async move {
                Ok(LibraryPage {
                    entries,
                    total_count: None,
                })
            }
        });
    }
}

async fn scans(service: &LibraryService) -> Vec<MarketplaceScan> {
    let mut receiver = service.fetch_other_marketplaces(Marketplace::US);
    let mut result = Vec::new();
    while let Some(scan) = receiver.recv().await {
        result.push(scan);
    }
    result
}

#[tokio::test]
async fn fetches_all_pages_until_short_page() {
    let f = Fixture::new();
    f.script(
        vec![
            vec![entry("B1"), entry("B2")],
            vec![entry("B3"), entry("B4")],
            vec![entry("B5")],
        ],
        None,
    );
    assert_eq!(
        asins(&f.fetch(2).await.unwrap()),
        ["B1", "B2", "B3", "B4", "B5"]
    );
    assert_eq!(f.pages(), [1, 2, 3]);
}

#[tokio::test]
async fn full_last_page_needs_one_empty_page_without_total() {
    let f = Fixture::new();
    f.script(vec![vec![entry("B1"), entry("B2")]], None);
    f.fetch(2).await.unwrap();
    assert_eq!(f.pages(), [1, 2]);
}

#[tokio::test]
async fn total_count_stops_after_last_full_page() {
    let f = Fixture::new();
    f.script(
        vec![
            vec![entry("B1"), entry("B2")],
            vec![entry("B3"), entry("B4")],
        ],
        Some(4),
    );
    assert_eq!(f.fetch(2).await.unwrap().len(), 4);
    assert_eq!(f.pages(), [1, 2]);
}

#[tokio::test]
async fn empty_library() {
    let f = Fixture::new();
    f.script(vec![], None);
    assert!(f.fetch(2).await.unwrap().is_empty());
    assert_eq!(f.pages(), [1]);
}

#[tokio::test]
async fn repeated_page_terminates() {
    let f = Fixture::new();
    f.api.on_library(|_| async {
        Ok(LibraryPage {
            entries: vec![entry("B1"), entry("B2")],
            total_count: None,
        })
    });
    assert_eq!(asins(&f.fetch(2).await.unwrap()), ["B1", "B2"]);
    assert_eq!(f.pages(), [1, 2]);
}

#[tokio::test]
async fn overlapping_pages_are_deduplicated() {
    let f = Fixture::new();
    f.script(
        vec![
            vec![entry("B1"), entry("B2")],
            vec![entry("B2"), entry("B3")],
            vec![entry("B4")],
        ],
        None,
    );
    assert_eq!(asins(&f.fetch(2).await.unwrap()), ["B1", "B2", "B3", "B4"]);
}

#[tokio::test]
async fn filters_excluded_entries_across_pages() {
    let f = Fixture::new();
    let plus = entry_of(book("P1"), "SinglePartBook", "Product", true, vec![]);
    let episode = entry_of(book("E1"), "PodcastEpisode", "Podcast", false, vec![]);
    f.script(vec![vec![entry("B1"), plus], vec![episode]], None);
    assert_eq!(asins(&f.fetch(2).await.unwrap()), ["B1"]);
}

#[tokio::test]
async fn failing_page_fails_the_fetch() {
    let f = Fixture::new();
    f.api.on_library(|request| async move {
        if request.number == 2 {
            return Err(ExporterError::Http {
                status: 500,
                message: None,
            });
        }
        Ok(LibraryPage {
            entries: vec![entry("B1"), entry("B2")],
            total_count: None,
        })
    });
    assert_eq!(
        f.fetch(2).await,
        Err(ExporterError::Http {
            status: 500,
            message: None
        })
    );
}

#[tokio::test]
async fn rejected_token_is_refreshed_and_retried_once() {
    let f = Fixture::new();
    let calls = Arc::new(Locked::new(0));
    let counter = calls.clone();
    f.api.on_library(move |request| {
        counter.with(|count| *count += 1);
        async move {
            if request.session.access_token != "new" {
                return Err(ExporterError::AuthenticationRequired);
            }
            Ok(LibraryPage {
                entries: vec![entry("B1")],
                total_count: None,
            })
        }
    });
    f.api
        .on_refresh(|_| async { Ok(session_with_token("new")) });
    assert_eq!(asins(&f.fetch(2).await.unwrap()), ["B1"]);
    assert_eq!(calls.current(), 2);
    assert_eq!(f.api.refresh_count.current(), 1);
}

#[tokio::test]
async fn persistent_rejection_requires_sign_in() {
    let f = Fixture::new();
    f.api
        .on_library(|_| async { Err(ExporterError::AuthenticationRequired) });
    f.api
        .on_refresh(|_| async { Ok(session_with_token("new")) });
    let (service, auth) = f.make(2);
    assert_eq!(
        service.fetch_purchased_books(Marketplace::US).await,
        Err(ExporterError::AuthenticationRequired)
    );
    assert_eq!(
        auth.state(),
        AuthState::ReauthenticationRequired(Marketplace::US)
    );
}

#[tokio::test]
async fn home_library_uses_the_sessions_marketplace() {
    let f = Fixture::new();
    f.store.stored.set(Some(session_in(Marketplace::GERMANY)));
    f.script_stores(vec![
        (Marketplace::GERMANY, vec!["D1"]),
        (Marketplace::US, vec!["U1"]),
    ]);
    let (marketplace, books) = f.make(100).0.fetch_home_library().await.unwrap();
    assert_eq!(marketplace, Marketplace::GERMANY);
    assert_eq!(asins(&books), ["D1"]);
    assert!(
        books
            .iter()
            .all(|book| book.marketplace == Marketplace::GERMANY)
    );
    assert_eq!(
        f.api.requested_marketplaces.current(),
        [Marketplace::GERMANY]
    );
}

fn titles(asin: &str, marketplace: Marketplace) -> Audiobook {
    book_in(asin, &format!("Book {asin}"), marketplace)
}

#[tokio::test]
async fn checks_every_other_store_once_in_order() {
    let f = Fixture::new();
    f.script_stores(vec![
        (Marketplace::US, vec!["U1"]),
        (Marketplace::UK, vec!["K1", "K2"]),
        (Marketplace::JAPAN, vec!["J1"]),
    ]);
    let result = scans(&f.make(100).0).await;
    let others: Vec<Marketplace> = Marketplace::ALL
        .into_iter()
        .filter(|m| *m != Marketplace::US)
        .collect();
    assert_eq!(
        result
            .iter()
            .map(MarketplaceScan::marketplace)
            .collect::<Vec<_>>(),
        others
    );
    assert_eq!(f.api.requested_marketplaces.current(), others);
    assert_eq!(
        result[0],
        MarketplaceScan::Titles(
            Marketplace::UK,
            vec![titles("K1", Marketplace::UK), titles("K2", Marketplace::UK)]
        )
    );
    assert_eq!(
        *result.last().unwrap(),
        MarketplaceScan::Titles(Marketplace::JAPAN, vec![titles("J1", Marketplace::JAPAN)])
    );
    assert_eq!(result[1], MarketplaceScan::Empty(Marketplace::GERMANY));
}

#[tokio::test]
async fn excluded_entries_leave_a_store_empty() {
    let f = Fixture::new();
    f.api.on_library(|request| async move {
        let entries = if request.marketplace == Marketplace::UK {
            vec![entry_of(
                book_in("P1", "x", Marketplace::UK),
                "SinglePartBook",
                "Product",
                true,
                vec![],
            )]
        } else {
            vec![]
        };
        Ok(LibraryPage {
            entries,
            total_count: None,
        })
    });
    assert_eq!(
        scans(&f.make(100).0).await[0],
        MarketplaceScan::Empty(Marketplace::UK)
    );
}

#[tokio::test]
async fn rejected_store_fails_alone_and_keeps_the_session() {
    let f = Fixture::new();
    f.api.on_library(|request| async move {
        match request.marketplace.id {
            "uk" => Err(ExporterError::AuthenticationRequired),
            "de" => Err(ExporterError::Http {
                status: 503,
                message: None,
            }),
            "jp" => Ok(LibraryPage {
                entries: vec![entry_in("J1", Marketplace::JAPAN)],
                total_count: None,
            }),
            _ => Ok(LibraryPage {
                entries: vec![],
                total_count: None,
            }),
        }
    });
    let (service, auth) = f.make(100);
    let result = scans(&service).await;
    assert_eq!(
        result[0],
        MarketplaceScan::Failed(Marketplace::UK, ExporterError::AuthenticationRequired)
    );
    assert_eq!(
        result[1],
        MarketplaceScan::Failed(
            Marketplace::GERMANY,
            ExporterError::Http {
                status: 503,
                message: None
            }
        )
    );
    assert_eq!(
        *result.last().unwrap(),
        MarketplaceScan::Titles(Marketplace::JAPAN, vec![titles("J1", Marketplace::JAPAN)])
    );
    // Regression guard: a foreign store's 401 must not end the session or wipe the credentials.
    assert_eq!(auth.state(), AuthState::SignedIn(session()));
    assert_eq!(f.store.stored.current(), Some(session()));
    assert_eq!(f.api.refresh_count.current(), 0);
}

#[tokio::test]
async fn expired_token_is_refreshed_once_for_all_stores() {
    let f = Fixture::new();
    f.clock.advance(3600);
    f.api.on_refresh(|_| async {
        Ok(AudibleSession {
            access_token: "new".into(),
            access_token_expires_at: seconds_after_reference(7200),
            ..session()
        })
    });
    let tokens = Arc::new(Locked::new(Vec::new()));
    let record = tokens.clone();
    f.api.on_library(move |request| {
        record.with(|all: &mut Vec<String>| all.push(request.session.access_token));
        async {
            Ok(LibraryPage {
                entries: vec![],
                total_count: None,
            })
        }
    });
    scans(&f.make(100).0).await;
    assert_eq!(f.api.refresh_count.current(), 1);
    let tokens = tokens.current();
    assert_eq!(tokens.len(), Marketplace::ALL.len() - 1);
    assert!(tokens.iter().all(|token| token == "new"));
}

#[tokio::test]
async fn stops_when_the_consumer_is_cancelled() {
    let f = Fixture::new();
    let gate = AsyncGate::default();
    let held = gate.clone();
    f.api.on_library(move |_| {
        let held = held.clone();
        async move {
            held.wait().await;
            Ok(LibraryPage {
                entries: vec![],
                total_count: None,
            })
        }
    });
    let receiver = f.make(100).0.fetch_other_marketplaces(Marketplace::US);
    let api = f.api.clone();
    wait_until(move || api.requested_marketplaces.current().len() == 1).await;
    drop(receiver);
    gate.open();
    // Give a runaway producer the chance to ask another store.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.api.requested_marketplaces.current().len(), 1);
}

fn search_library() -> Vec<Audiobook> {
    vec![
        Audiobook {
            authors: vec!["Victor Hugo".into()],
            narrators: vec!["Narrator One".into()],
            ..Audiobook::new("B1", Marketplace::US, "Les Misérables")
        },
        Audiobook {
            subtitle: Some("Book One".into()),
            authors: vec!["Frank Herbert".into()],
            series_title: Some("Dune Chronicles".into()),
            ..Audiobook::new("B2", Marketplace::US, "Dune")
        },
        Audiobook {
            authors: vec!["Andy Weir".into()],
            narrators: vec!["Ray Porter".into()],
            ..Audiobook::new("B3", Marketplace::US, "Project Hail Mary")
        },
    ]
}

#[test]
fn empty_query_returns_everything() {
    assert_eq!(
        asins(&search_books(&search_library(), "  ")),
        ["B1", "B2", "B3"]
    );
}

#[test]
fn matches_all_terms_across_fields() {
    let cases: [(&str, &[&str]); 8] = [
        ("miserables", &["B1"]),
        ("HUGO", &["B1"]),
        ("ray porter", &["B3"]),
        ("chronicles", &["B2"]),
        ("book one", &["B2"]),
        ("b2", &["B2"]),
        ("dune weir", &[]),
        ("o", &["B1", "B2", "B3"]),
    ];
    for (query, expected) in cases {
        assert_eq!(
            asins(&search_books(&search_library(), query)),
            expected,
            "{query}"
        );
    }
}

fn selection_books() -> Vec<Audiobook> {
    (1..=4).map(|n| book(&format!("B{n}"))).collect()
}

fn set(asins: &[&str]) -> HashSet<String> {
    asins.iter().map(|asin| asin.to_string()).collect()
}

#[test]
fn toggles_and_reports_in_library_order() {
    let mut selection = LibrarySelection::default();
    for asin in ["B3", "B1", "B2", "B2"] {
        selection.toggle(asin);
    }
    assert_eq!(
        asins(&selection.selected_books(&selection_books())),
        ["B1", "B3"]
    );
    assert_eq!(selection.len(), 2);
}

#[test]
fn selects_and_deselects_groups() {
    let books = selection_books();
    let mut selection = LibrarySelection::new(["B4"]);
    selection.select(&books[..2]);
    assert_eq!(*selection.asins(), set(&["B1", "B2", "B4"]));
    selection.deselect(&[books[0].clone(), books[3].clone()]);
    assert_eq!(*selection.asins(), set(&["B2"]));
    selection.set("B3", true);
    selection.clear();
    assert!(selection.is_empty());
}

#[test]
fn reconciles_against_refreshed_library() {
    let mut selection = LibrarySelection::new(["B1", "B3", "GONE"]);
    selection.reconcile(&selection_books()[..2]);
    assert_eq!(*selection.asins(), set(&["B1"]));
}
