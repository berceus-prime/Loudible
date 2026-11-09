//! The models on their own thread, seen as the UI thread sees them.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use loudible_core::*;
use loudible_presentation::*;
use loudible_test_support::*;
use support::*;
use tokio::sync::oneshot;

/// Blocks like the UI thread does, polling the published snapshot.
#[track_caller]
fn wait_for(handle: &AppHandle, condition: impl Fn(&AppSnapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition(&handle.snapshot()) {
        assert!(Instant::now() < deadline, "condition not met");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn started(f: &Fixture) -> (AppHandle, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let handle = start_app(f.dependencies(1), move || {
        counter.fetch_add(1, Ordering::Relaxed);
    })
    .unwrap();
    (handle, wakes)
}

fn loaded(snapshot: &AppSnapshot) -> bool {
    snapshot.library.visible_books.len() == 2 && !snapshot.library.is_checking_marketplaces
}

#[test]
fn publishes_the_loaded_library_and_wakes_the_ui() {
    let f = Fixture::new();
    let (handle, wakes) = started(&f);
    wait_for(&handle, loaded);
    let snapshot = handle.snapshot();
    assert!(snapshot.session.is_signed_in);
    assert_eq!(snapshot.library.summary, "2 books");
    assert_eq!(
        snapshot.library.statuses,
        [BookStatus::None, BookStatus::None]
    );
    assert!(wakes.load(Ordering::Relaxed) > 0);
}

#[test]
fn commands_change_later_snapshots_but_not_earlier_ones() {
    let f = Fixture::new();
    let (handle, _) = started(&f);
    wait_for(&handle, loaded);
    let before = handle.snapshot();

    handle.send(Command::SetSearchText("B2".into()));
    handle.send(Command::ToggleSelection("B2".into()));
    wait_for(&handle, |s| {
        s.library.visible_books.len() == 1 && s.library.selection.contains("B2")
    });

    assert_eq!(before.library.visible_books.len(), 2);
    assert!(before.library.selection.is_empty());
    let after = handle.snapshot();
    assert_eq!(after.library.selected_for_download, 1);
    assert_eq!(after.library.selected_for_export, 1);
}

#[test]
fn a_download_shows_progress_then_its_file_and_menu() {
    let f = Fixture::new();
    let gate = AsyncGate::default();
    let wait = gate.clone();
    f.downloader.on_download(move |_, destination, _| {
        let gate = wait.clone();
        async move {
            gate.wait().await;
            write(&destination, "")
        }
    });
    let (handle, _) = started(&f);
    wait_for(&handle, loaded);
    let target = handle.snapshot().library.visible_books[0].clone();

    handle.send(Command::Perform(
        BookAction::Download,
        Box::new(target.clone()),
    ));
    wait_for(&handle, |s| {
        matches!(s.library.statuses[0], BookStatus::Running { .. })
    });
    assert!(handle.snapshot().has_active_jobs);
    gate.open();
    wait_for(&handle, |s| s.library.statuses[0] == BookStatus::Downloaded);

    let snapshot = handle.snapshot();
    assert!(snapshot.downloaded_file(&target.asin).is_some());
    let actions: Vec<BookAction> = snapshot
        .menu_items(&target)
        .iter()
        .map(|i| i.action)
        .collect();
    assert_eq!(
        actions,
        [
            BookAction::Export,
            BookAction::ShowDownload,
            BookAction::DeleteDownload
        ]
    );
}

#[test]
fn suspend_replies_after_jobs_have_stopped() {
    let f = Fixture::new();
    f.downloader.on_download(|_, _, cancel| async move {
        cancel.cancelled().await;
        Err(ExporterError::Cancelled)
    });
    let (handle, _) = started(&f);
    wait_for(&handle, loaded);
    let target = handle.snapshot().library.visible_books[0].clone();
    handle.send(Command::Perform(BookAction::Download, Box::new(target)));
    wait_for(&handle, |s| {
        matches!(s.library.statuses[0], BookStatus::Running { .. })
    });

    let (reply, done) = oneshot::channel();
    handle.send(Command::Suspend(reply));
    done.blocking_recv().unwrap();
    wait_for(&handle, |s| !s.has_active_jobs);
}

#[test]
fn dismissing_a_notice_removes_it() {
    let f = Fixture::new();
    let (handle, _) = started(&f);
    wait_for(&handle, loaded);
    handle.send(Command::BrowserFailed("offline".into()));
    wait_for(&handle, |s| s.notices.len() == 1);
    let notice = handle.snapshot().notices[0].clone();
    assert_eq!(notice.error.title(), "Sign-in page couldn’t load");
    handle.send(Command::DismissNotice(notice.id));
    wait_for(&handle, |s| s.notices.is_empty());
}

#[test]
fn a_snapshot_can_be_read_from_another_thread() {
    fn assert_shareable<T: Send + Sync>() {}
    assert_shareable::<AppSnapshot>();
    assert_shareable::<AppHandle>();
}

#[test]
fn dropping_the_handle_stops_the_model_thread() {
    let f = Fixture::new();
    let (handle, _) = started(&f);
    wait_for(&handle, loaded);
    let counted = Arc::strong_count(&f.api);
    drop(handle);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Arc::strong_count(&f.api) >= counted {
        assert!(Instant::now() < deadline, "the model thread kept running");
        std::thread::sleep(Duration::from_millis(2));
    }
}
