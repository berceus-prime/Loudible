//! `LibraryViewModel`.

mod support;

use std::collections::HashSet;
use std::sync::Arc;

use loudible_core::*;
use loudible_presentation::*;
use loudible_test_support::*;
use support::*;

struct Setup {
    api: Arc<MockAudibleApi>,
    store: Arc<InMemoryCredentialStore>,
}

impl Setup {
    fn new() -> Self {
        Self {
            api: Arc::default(),
            store: Arc::new(InMemoryCredentialStore::with(session())),
        }
    }

    fn model(&self, signed_in: bool) -> LibraryViewModel {
        let auth = AuthenticationCoordinator::new(
            self.api.clone(),
            self.store.clone(),
            Arc::new(TestClock::default()),
        );
        auth.restore().unwrap();
        let service = LibraryService::new(self.api.clone(), auth, 100);
        let model = LibraryViewModel::new(service, Changes::default());
        model.set_signed_in(signed_in);
        model
    }

    /// The home (US) store holds `asins`; `foreign` maps other stores to theirs.
    fn script(&self, asins: &[&'static str], foreign: &[(Marketplace, &[&'static str])]) {
        let asins = asins.to_vec();
        let foreign: Vec<(Marketplace, Vec<&'static str>)> = foreign
            .iter()
            .map(|(m, held)| (*m, held.to_vec()))
            .collect();
        self.api.on_library(move |request| {
            let held = if request.marketplace == Marketplace::US {
                asins.clone()
            } else {
                foreign
                    .iter()
                    .find(|(m, _)| *m == request.marketplace)
                    .map(|(_, held)| held.clone())
                    .unwrap_or_default()
            };
            let marketplace = request.marketplace;
            async move {
                Ok(page(
                    held.iter().map(|a| entry_in(a, marketplace)).collect(),
                ))
            }
        });
    }
}

fn asins(books: &[Audiobook]) -> Vec<&str> {
    books.iter().map(|book| book.asin.as_str()).collect()
}

fn selected(model: &LibraryViewModel) -> HashSet<String> {
    model.selection().asins().clone()
}

fn set_of(asins: &[&str]) -> HashSet<String> {
    asins.iter().map(ToString::to_string).collect()
}

fn unavailable() -> ExporterError {
    ExporterError::Http {
        status: 503,
        message: None,
    }
}

#[test]
fn starts_idle_with_actions_disabled_when_signed_out() {
    local(async {
        let setup = Setup::new();
        let model = setup.model(false);
        assert_eq!(model.load_state(), LoadState::Idle);
        assert!(!model.can_refresh());
        assert!(!model.can_select());
        model.refresh().await;
        assert!(setup.api.requested_pages.current().is_empty());
    });
}

#[test]
fn shows_loading_then_loaded() {
    local(async {
        let setup = Setup::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        setup.api.on_library(move |request| {
            let gate = wait.clone();
            async move {
                gate.wait().await;
                Ok(page(if request.marketplace == Marketplace::US {
                    vec![entry("B1")]
                } else {
                    vec![]
                }))
            }
        });
        let model = setup.model(true);
        let refreshing = model.clone();
        let refresh = tokio::task::spawn_local(async move { refreshing.refresh().await });
        wait_until(|| model.is_loading()).await;
        assert!(!model.can_refresh());
        gate.open();
        refresh.await.unwrap();
        assert_eq!(model.load_state(), LoadState::Loaded);
        assert_eq!(asins(&model.books()), ["B1"]);
        assert!(model.can_refresh());
        assert!(model.can_select());
    });
}

#[test]
fn failed_refresh_keeps_rows_and_shows_error() {
    local(async {
        let setup = Setup::new();
        setup.script(&["B1", "B2"], &[]);
        let model = setup.model(true);
        model.refresh().await;
        model.toggle_selection("B1");

        setup.api.on_library(|_| async { Err(unavailable()) });
        model.refresh().await;
        assert_eq!(asins(&model.books()), ["B1", "B2"]);
        assert!(model.is_selected("B1"));
        assert!(model.error_message().unwrap().contains("503"));
        assert!(model.can_refresh());
    });
}

#[test]
fn successful_refresh_reconciles_selection() {
    local(async {
        let setup = Setup::new();
        setup.script(&["B1", "B2", "B3"], &[]);
        let model = setup.model(true);
        model.refresh().await;
        model.set_selection(["B1", "B3"]);

        setup.script(&["B2", "B3"], &[]);
        model.refresh().await;
        assert_eq!(selected(&model), set_of(&["B3"]));
        assert!(model.error_message().is_none());
    });
}

#[test]
fn search_filters_visible_books_and_select_all_uses_them() {
    local(async {
        let setup = Setup::new();
        setup.api.on_library(|request| async move {
            Ok(page(if request.marketplace == Marketplace::US {
                vec![
                    titled("B1", "Alpha"),
                    titled("B2", "Beta"),
                    titled("B3", "Alphabet"),
                ]
            } else {
                vec![]
            }))
        });
        let model = setup.model(true);
        model.refresh().await;
        model.set_search_text("alpha");
        assert_eq!(asins(&model.visible_books()), ["B1", "B3"]);
        model.select_all_visible();
        assert_eq!(asins(&model.selected_books()), ["B1", "B3"]);
        assert_eq!(model.summary(), "2 of 3 books, 2 selected");

        model.set_search_text("");
        model.toggle_selection("B2");
        model.set_search_text("alphabet");
        model.deselect_all_visible();
        assert_eq!(selected(&model), set_of(&["B1", "B2"]));
    });
}

#[test]
fn ignores_unknown_selections() {
    local(async {
        let setup = Setup::new();
        setup.script(&["B1"], &[]);
        let model = setup.model(true);
        model.refresh().await;
        model.toggle_selection("NOPE");
        model.set_selection(["B1", "NOPE"]);
        assert_eq!(selected(&model), set_of(&["B1"]));
    });
}

#[test]
fn reset_clears_everything() {
    local(async {
        let setup = Setup::new();
        setup.script(&["B1"], &[]);
        let model = setup.model(true);
        model.refresh().await;
        model.toggle_selection("B1");
        model.set_search_text("x");
        model.reset();
        assert!(model.books().is_empty());
        assert!(model.selection().is_empty());
        assert!(model.search_text().is_empty());
        assert_eq!(model.load_state(), LoadState::Idle);
    });
}

#[test]
fn hides_books_with_morning_in_the_title() {
    local(async {
        let setup = Setup::new();
        setup.api.on_library(|request| async move {
            Ok(page(if request.marketplace == Marketplace::US {
                vec![
                    titled("B1", "Good morning, Midnight"),
                    titled("B2", "Evening"),
                ]
            } else {
                vec![]
            }))
        });
        let model = setup.model(true);
        model.refresh().await;
        model.apply_local(vec![book_in("B3", "MORNINGS", Marketplace::US)]);
        assert_eq!(asins(&model.books()), ["B2"]);
    });
}

#[test]
fn downloaded_book_dropped_by_refresh_stays_visible() {
    local(async {
        let setup = Setup::new();
        setup.script(&["B1", "B2"], &[]);
        let model = setup.model(true);
        model.refresh().await;
        model.apply_local(vec![book_in("B2", "Kept", Marketplace::US)]);
        assert_eq!(asins(&model.books()), ["B1", "B2"]);
        assert_eq!(model.books()[1].title, "Book B2"); // the library's version wins while it is there
        model.set_selection(["B2"]);

        setup.script(&["B1"], &[]);
        model.refresh().await;
        assert_eq!(asins(&model.books()), ["B1", "B2"]);
        assert_eq!(model.books()[1].title, "Kept");
        assert!(!model.is_in_library("B2"));
        assert!(model.is_selected("B2"));
        model.set_search_text("kept");
        assert_eq!(asins(&model.visible_books()), ["B2"]);
    });
}

#[test]
fn downloaded_books_are_hidden_when_signed_out() {
    local(async {
        let setup = Setup::new();
        let model = setup.model(false);
        model.apply_local(vec![book("B9")]);
        assert!(model.books().is_empty());
        model.set_signed_in(true);
        assert_eq!(asins(&model.books()), ["B9"]);
        model.reset();
        assert!(model.books().is_empty());
    });
}

#[test]
fn local_only_books_sort_naturally() {
    local(async {
        let setup = Setup::new();
        let model = setup.model(true);
        let local_book = |asin, title| book_in(asin, title, Marketplace::US);
        model.apply_local(vec![
            local_book("A", "Book 10"),
            local_book("B", "Book 2"),
            local_book("C", "apple"),
        ]);
        assert_eq!(asins(&model.books()), ["C", "B", "A"]);
    });
}

#[test]
fn merges_titles_from_other_stores_and_tags_them() {
    local(async {
        let setup = Setup::new();
        setup.script(
            &["U1", "U2"],
            &[
                (Marketplace::UK, &["K1", "U1"]),
                (Marketplace::JAPAN, &["J1"]),
            ],
        );
        let model = setup.model(true);
        model.refresh().await;
        let books = model.books();
        assert_eq!(asins(&books), ["U1", "U2", "K1", "J1"]);
        assert_eq!(books[0].marketplace, Marketplace::US); // the home copy wins
        assert_eq!(model.home_marketplace(), Some(Marketplace::US));
        assert_eq!(model.marketplace_badge(&books[0]), None);
        assert_eq!(model.marketplace_badge(&books[2]).unwrap(), "UK");
        assert!(model.is_in_library("J1"));
        assert_eq!(
            model.other_marketplaces(),
            [
                MarketplaceCount {
                    marketplace: Marketplace::UK,
                    count: 1
                },
                MarketplaceCount {
                    marketplace: Marketplace::JAPAN,
                    count: 1
                },
            ]
        );
        assert_eq!(model.summary(), "4 books · also in United Kingdom, Japan");
        assert!(!model.is_checking_marketplaces());
        assert!(model.can_refresh());
    });
}

#[test]
fn home_books_show_before_other_stores_answer() {
    local(async {
        let setup = Setup::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        setup.api.on_library(move |request| {
            let gate = wait.clone();
            async move {
                let marketplace = request.marketplace;
                if marketplace == Marketplace::US {
                    return Ok(page(vec![entry("U1")]));
                }
                gate.wait().await;
                Ok(page(if marketplace == Marketplace::UK {
                    vec![entry_in("K1", marketplace)]
                } else {
                    vec![]
                }))
            }
        });
        let model = setup.model(true);
        let refreshing = model.clone();
        let refresh = tokio::task::spawn_local(async move { refreshing.refresh().await });
        wait_until(|| model.is_checking_marketplaces()).await;
        assert_eq!(model.load_state(), LoadState::Loaded);
        assert_eq!(asins(&model.books()), ["U1"]);
        assert!(!model.can_refresh());
        gate.open();
        refresh.await.unwrap();
        assert_eq!(asins(&model.books()), ["U1", "K1"]);
        assert!(!model.is_checking_marketplaces());
    });
}

#[test]
fn selected_title_from_another_store_survives_refresh() {
    local(async {
        let setup = Setup::new();
        setup.script(&["U1"], &[(Marketplace::UK, &["K1"])]);
        let model = setup.model(true);
        model.refresh().await;
        model.set_selection(["U1", "K1"]);

        model.refresh().await;
        assert_eq!(selected(&model), set_of(&["U1", "K1"]));

        setup.script(&["U1"], &[]);
        model.refresh().await;
        assert_eq!(selected(&model), set_of(&["U1"]));
        assert!(model.other_marketplaces().is_empty());
    });
}

#[test]
fn stores_that_fail_are_a_notice_not_an_error() {
    local(async {
        let setup = Setup::new();
        let model = setup.model(true);
        setup.script(&["U1"], &[(Marketplace::INDIA, &["I1"])]);
        model.refresh().await;
        assert_eq!(asins(&model.books()), ["U1", "I1"]);

        setup.api.on_library(|request| async move {
            match request.marketplace {
                Marketplace::US => Ok(page(vec![entry("U1")])),
                Marketplace::JAPAN | Marketplace::INDIA => {
                    Err(ExporterError::AuthenticationRequired)
                }
                _ => Ok(page(vec![])),
            }
        });
        model.refresh().await;
        assert!(model.error_message().is_none());
        assert_eq!(model.load_state(), LoadState::Loaded);
        assert_eq!(
            model.unchecked_marketplaces(),
            [Marketplace::INDIA, Marketplace::JAPAN]
        );
        assert_eq!(
            model.marketplace_notice().unwrap(),
            "Couldn’t check the Audible India, Japan marketplaces for more titles."
        );
        // What India held last time stays until it answers again.
        assert_eq!(asins(&model.books()), ["U1", "I1"]);
        assert!(model.is_signed_in());
    });
}

#[test]
fn failed_home_load_skips_other_stores() {
    local(async {
        let setup = Setup::new();
        setup.api.on_library(|_| async { Err(unavailable()) });
        let model = setup.model(true);
        model.refresh().await;
        assert!(model.error_message().unwrap().contains("503"));
        assert_eq!(
            setup.api.requested_marketplaces.current(),
            [Marketplace::US]
        );
    });
}

#[test]
fn reset_cancels_the_marketplace_check() {
    local(async {
        let setup = Setup::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        setup.api.on_library(move |request| {
            let gate = wait.clone();
            async move {
                let marketplace = request.marketplace;
                if marketplace == Marketplace::US {
                    return Ok(page(vec![entry("U1")]));
                }
                gate.wait().await;
                Ok(page(vec![entry_in(
                    &format!("X-{}", marketplace.id),
                    marketplace,
                )]))
            }
        });
        let model = setup.model(true);
        let refreshing = model.clone();
        let refresh = tokio::task::spawn_local(async move { refreshing.refresh().await });
        wait_until(|| model.is_checking_marketplaces()).await;
        wait_until(|| setup.api.requested_marketplaces.current().len() == 2).await;
        model.reset();
        assert!(!model.is_checking_marketplaces());
        gate.open();
        refresh.await.unwrap();
        tokio::task::yield_now().await;
        assert!(model.books().is_empty());
        assert!(model.other_marketplaces().is_empty());
        assert_eq!(setup.api.requested_marketplaces.current().len(), 2);
    });
}
