//! The real window over the real engine: only Audible, the downloader and FFmpeg are replaced.

use std::sync::Arc;
use std::time::{Duration, Instant};

use loudible_core::*;
use loudible_presentation::{AppDependencies, start_app};
use loudible_test_support::{
    AsyncGate, InMemoryCredentialStore, MockAudibleApi, MockConverter, MockDownloader,
    TemporaryDirectory, TestClock, entry, license, session, us_callback, write,
};
use skia_safe::{Point, Size, surfaces};
use skia_ui::{
    model::{AppModel, BookAction as ViewAction, BookStatus as ViewStatus, LiveApp},
    shell::app_view::AppView,
    stage::{Effect, Key, Screen, Stage},
    theme::Theme,
    ui::{Action, Field},
};

const SIZE: Size = Size::new(1100.0, 652.0);

struct Flow {
    stage: Stage<AppView<LiveApp>>,
    api: Arc<MockAudibleApi>,
    downloader: Arc<MockDownloader>,
    store: Arc<InMemoryCredentialStore>,
    downloads: LocalDownloads,
    _work: TemporaryDirectory,
}

fn flow(signed_in: bool) -> Flow {
    let work = TemporaryDirectory::default();
    let api = Arc::new(MockAudibleApi::default());
    api.on_license(|asin, _| async move { Ok(license(&asin)) });
    api.on_library(|request| async move {
        let entries = if request.marketplace == Marketplace::US {
            vec![entry("B1"), entry("B2")]
        } else {
            vec![]
        };
        Ok(LibraryPage {
            entries,
            total_count: None,
        })
    });
    let store = Arc::new(InMemoryCredentialStore::default());
    if signed_in {
        store.stored.set(Some(session()));
    }
    let downloader = Arc::new(MockDownloader::default());
    let auth =
        AuthenticationCoordinator::new(api.clone(), store.clone(), Arc::new(TestClock::default()));
    let downloads = LocalDownloads::with_trash(
        work.join("Downloads"),
        work.join("Manifests"),
        Arc::new(|path| std::fs::remove_file(path)),
    );
    let queue = ExportQueue::new(
        auth.clone(),
        api.clone(),
        downloader.clone(),
        Arc::new(MockConverter::default()),
        downloads.clone(),
    );
    let handle = start_app(
        AppDependencies {
            library: LibraryService::new(api.clone(), auth.clone(), 100),
            auth,
            queue,
            export_folder: downloads.folder.clone(),
            downloads: downloads.clone(),
        },
        || {},
    )
    .unwrap();
    Flow {
        stage: Stage::new(AppView::with_app(LiveApp::new(handle)), Theme::LIGHT),
        api,
        downloader,
        store,
        downloads,
        _work: work,
    }
}

impl Flow {
    fn frame(&mut self) {
        let mut surface = surfaces::raster_n32_premul((1100, 652)).unwrap();
        self.stage.draw(surface.canvas(), SIZE);
    }

    #[track_caller]
    fn wait_for(&mut self, condition: impl Fn(&LiveApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.stage.advance(Duration::from_millis(16));
            self.frame();
            if condition(&self.stage.screen.app) {
                return;
            }
            assert!(Instant::now() < deadline, "condition not met");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn loaded(&mut self) {
        self.wait_for(|app| app.visible_books().len() == 2 && !app.is_checking_marketplaces());
    }

    fn click(&mut self, matches: impl Fn(&Action) -> bool) -> Option<Effect> {
        self.frame();
        let hit = self
            .stage
            .hits()
            .iter()
            .rev()
            .find(|hit| matches(&hit.action))
            .expect("control is on screen");
        let point = Point::new(hit.rect.center_x(), hit.rect.center_y());
        let effect = self.stage.click(point);
        self.frame();
        effect
    }

    fn status(&self, index: usize) -> ViewStatus {
        let app = &self.stage.screen.app;
        app.tile(app.visible_books()[index]).status.clone()
    }

    fn open_menu(&mut self, asin: &'static str) {
        self.click(|a| matches!(a, Action::OpenBookMenu(id, _) if &**id == asin));
    }
}

#[test]
fn a_saved_session_shows_the_library() {
    let mut flow = flow(true);
    flow.loaded();
    assert!(flow.stage.screen.app.is_signed_in());
    assert_eq!(&*flow.stage.screen.app.visible_books()[0].asin, "B1");
}

#[test]
fn signing_in_uses_the_real_login_page_and_callback() {
    let mut flow = flow(false);
    flow.api.on_register(|_, _| async { Ok(session()) });
    flow.wait_for(|app| app.has_restored());
    flow.click(|a| *a == Action::SignIn);
    flow.wait_for(|app| app.pending_login().is_some());

    let page = flow
        .stage
        .screen
        .browser()
        .expect("the sheet asks for a browser")
        .page;
    assert!(page.url.contains("synthetic=1"));
    assert_eq!(page.callback_host, "www.amazon.com");
    assert!(page.is_callback(&us_callback()));

    flow.stage.screen.browser_callback(us_callback());
    flow.loaded();
    assert!(flow.stage.screen.app.is_signed_in());
    assert!(flow.stage.screen.browser().is_none());
    assert_eq!(flow.store.stored.current(), Some(session()));
}

#[test]
fn the_dots_menu_downloads_and_can_cancel_that_job() {
    let mut flow = flow(true);
    let gate = AsyncGate::default();
    let wait = gate.clone();
    flow.downloader.on_download(move |_, destination, cancel| {
        let gate = wait.clone();
        async move {
            tokio::select! {
                () = gate.wait() => write(&destination, ""),
                () = cancel.cancelled() => Err(ExporterError::Cancelled),
            }
        }
    });
    flow.loaded();
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Download, id) if &**id == "B1"));
    flow.wait_for(|app| {
        matches!(
            app.tile(app.visible_books()[0]).status,
            ViewStatus::Running { .. }
        )
    });

    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Cancel(_), id) if &**id == "B1"));
    flow.wait_for(|app| {
        app.tile(app.visible_books()[0]).status
            != ViewStatus::Queued(skia_ui::model::JobKind::Download)
            && !matches!(
                app.tile(app.visible_books()[0]).status,
                ViewStatus::Running { .. }
            )
    });

    flow.stage.screen.handle(
        Action::Perform(ViewAction::Download, "B2".into()),
        Point::default(),
        &skia_ui::text::TextSystem::default(),
    );
    flow.wait_for(|app| {
        matches!(
            app.tile(app.visible_books()[1]).status,
            ViewStatus::Running { .. }
        )
    });
    gate.open();
    flow.wait_for(|app| app.tile(app.visible_books()[1]).status == ViewStatus::Downloaded);
    assert_eq!(flow.status(1), ViewStatus::Downloaded);
}

#[test]
fn a_finished_download_can_be_shown_in_its_folder() {
    let mut flow = flow(true);
    flow.loaded();
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Download, id) if &**id == "B1"));
    flow.wait_for(|app| app.tile(app.visible_books()[0]).status == ViewStatus::Downloaded);
    flow.open_menu("B1");
    let effect = flow.click(|a| matches!(a, Action::Perform(ViewAction::ShowDownload, _)));
    assert!(
        matches!(&effect, Some(Effect::Reveal(path)) if path.to_string_lossy().contains("[B1]")),
        "{effect:?}"
    );
}

#[test]
fn search_is_folded_by_the_engine() {
    let mut flow = flow(true);
    flow.loaded();
    flow.click(|a| matches!(a, Action::FocusField(Field::Search, _)));
    flow.stage.key(Key::Text("b2".into()));
    flow.wait_for(|app| app.visible_books().len() == 1);
    assert_eq!(&*flow.stage.screen.app.visible_books()[0].asin, "B2");
}

#[test]
fn selection_reaches_the_toolbar_and_clears() {
    let mut flow = flow(true);
    flow.loaded();
    flow.click(|a| matches!(a, Action::ToggleBook(id) if &**id == "B1"));
    flow.wait_for(|app| app.selected_count() == 1);
    assert_eq!(
        flow.stage
            .screen
            .app
            .selection_offering(ViewAction::Download),
        1
    );
    flow.click(|a| *a == Action::OnSelection(ViewAction::Download));
    flow.wait_for(|app| app.selected_count() == 0);
    flow.wait_for(|app| app.tile(app.visible_books()[0]).status == ViewStatus::Downloaded);
}

#[test]
fn failures_become_toasts_that_copy_and_dismiss() {
    let mut flow = flow(true);
    flow.downloader
        .on_download(|_, _, _| async { Err(ExporterError::Download("HTTP 500".into())) });
    flow.loaded();
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Download, id) if &**id == "B1"));
    flow.wait_for(|app| app.toasts().len() == 1);
    let id = flow.stage.screen.app.toasts()[0].id;
    let effect = flow.click(|a| *a == Action::CopyToast(id));
    assert!(matches!(effect, Some(Effect::Copy(text)) if text.contains("HTTP 500")));
    flow.wait_for(|app| app.toasts()[0].is_copied);
    flow.click(|a| *a == Action::DismissToast(id));
    flow.wait_for(|app| app.toasts().is_empty());
}

#[test]
fn deleting_asks_first_and_then_removes_the_file() {
    let mut flow = flow(true);
    flow.loaded();
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Download, id) if &**id == "B1"));
    flow.wait_for(|app| app.tile(app.visible_books()[0]).status == ViewStatus::Downloaded);
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::DeleteDownload, _)));
    assert_eq!(flow.status(0), ViewStatus::Downloaded);
    flow.click(|a| *a == Action::ConfirmDelete);
    flow.wait_for(|app| app.tile(app.visible_books()[0]).status == ViewStatus::None);
}

#[test]
fn signing_out_empties_the_window() {
    let mut flow = flow(true);
    flow.loaded();
    flow.click(|a| *a == Action::ToggleAccount);
    flow.click(|a| *a == Action::SignOut);
    flow.wait_for(|app| app.is_signed_out_and_empty());
    assert_eq!(flow.store.stored.current(), None);
}

fn stops_when_cancelled(flow: &Flow) {
    flow.downloader.on_download(|_, _, cancel| async move {
        cancel.cancelled().await;
        Err(ExporterError::Cancelled)
    });
}

fn start_download(flow: &mut Flow, asin: &'static str) {
    flow.open_menu(asin);
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Download, id) if &**id == asin));
    flow.wait_for(|app| app.has_active_jobs());
}

#[test]
fn an_idle_app_closes_at_once() {
    let mut flow = flow(true);
    flow.loaded();
    assert!(flow.stage.screen.close_requested());
}

#[test]
fn closing_during_a_download_stops_it_keeping_its_progress() {
    let mut flow = flow(true);
    stops_when_cancelled(&flow);
    flow.loaded();
    start_download(&mut flow, "B1");
    flow.wait_for(|app| {
        matches!(
            app.tile(app.visible_books()[0]).status,
            ViewStatus::Running { .. }
        )
    });

    assert!(!flow.stage.screen.close_requested());
    let waited = Instant::now();
    while !flow.stage.screen.is_ready_to_close() {
        flow.stage.advance(Duration::from_millis(16));
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "never finished stopping"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!flow.downloads.partial_file_path("B1").exists());
}

#[test]
fn closing_with_work_that_would_be_lost_asks_first() {
    let mut flow = flow(true);
    stops_when_cancelled(&flow);
    flow.loaded();
    flow.open_menu("B1");
    flow.click(|a| matches!(a, Action::Perform(ViewAction::Export, id) if &**id == "B1"));
    flow.wait_for(|app| app.has_work_suspend_loses());

    assert!(!flow.stage.screen.close_requested());
    flow.click(|a| *a == Action::CancelQuit);
    assert!(!flow.stage.screen.is_ready_to_close());
    assert!(flow.stage.screen.app.has_active_jobs());

    assert!(!flow.stage.screen.close_requested());
    flow.stage.key(Key::Escape);
    assert!(flow.stage.screen.app.has_active_jobs());

    assert!(!flow.stage.screen.close_requested());
    flow.click(|a| *a == Action::ConfirmQuit);
    let started = Instant::now();
    while !flow.stage.screen.is_ready_to_close() {
        flow.stage.advance(Duration::from_millis(16));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "never finished stopping"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn coming_to_the_front_notices_files_changed_meanwhile() {
    let mut flow = flow(true);
    flow.loaded();
    flow.downloads.ensure_folder().unwrap();
    std::fs::write(flow.downloads.folder.join("Book B1 [B1].aaxc"), "enc").unwrap();
    assert_eq!(flow.status(0), ViewStatus::None);
    flow.stage.screen.window_focused();
    flow.wait_for(|app| app.tile(app.visible_books()[0]).status == ViewStatus::Downloaded);
}
