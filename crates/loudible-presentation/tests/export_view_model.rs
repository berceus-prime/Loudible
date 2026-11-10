//! `ExportViewModel`.

mod support;

use loudible_core::*;
use loudible_presentation::*;
use loudible_test_support::*;
use support::*;
use tokio::task::spawn_local;

fn asins_of<V>(map: &std::collections::HashMap<String, V>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

fn actions(items: &[BookMenuItem]) -> Vec<BookAction> {
    items.iter().map(|item| item.action).collect()
}

fn export_all(app: &AppViewModel) {
    for book in app.library().books() {
        app.export().export(&[book]);
    }
}

fn kept() -> TransferProgress {
    TransferProgress::new(50, 100)
}

fn library_asins(app: &AppViewModel) -> Vec<String> {
    app.library().books().into_iter().map(|b| b.asin).collect()
}

fn titled_book(asin: &str, title: &str) -> Audiobook {
    book_in(asin, title, Marketplace::US)
}

#[test]
fn signed_in_start_loads_library() {
    local(async {
        let f = Fixture::new();
        let (app, _) = f.make_app().await;
        assert!(app.session().is_signed_in());
        assert_eq!(library_asins(&app), ["B1", "B2"]);
    });
}

#[test]
fn exports_into_the_downloads_folder() {
    local(async {
        let f = Fixture::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        f.converter.on_convert(move |request, _| {
            let gate = wait.clone();
            async move {
                gate.wait().await;
                write(&request.output, "")
            }
        });
        let (app, queue) = f.make_app().await;
        export_all(&app);

        wait_until(|| app.export().is_running()).await;
        gate.open();
        queue.wait_until_finished().await;
        wait_until(|| app.exported_files().len() == 2).await;
        assert!(!app.export().is_running());
        assert!(
            app.exported_files()
                .values()
                .all(|path| path.parent() == Some(f.downloads.folder.as_path()))
        );
        // The M4B files are not mistaken for downloads.
        assert_eq!(asins_of(&app.downloaded_files()), ["B1", "B2"]);
    });
}

#[test]
fn cancel_removes_all_books() {
    local(async {
        let f = Fixture::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        f.converter.on_convert(move |request, cancel| {
            let gate = wait.clone();
            async move {
                gate.wait().await;
                check(&cancel)?;
                write(&request.output, "")
            }
        });
        let (app, queue) = f.make_app().await;
        export_all(&app);
        wait_until(|| f.converter.requests.current().len() == 1).await;
        let export = app.export().clone();
        let cancelling = spawn_local(async move { export.cancel_all().await });
        tokio::task::yield_now().await;
        gate.open();
        cancelling.await.unwrap();
        queue.wait_until_finished().await;
        assert!(!app.export().is_running());
        assert!(app.export().jobs().is_empty());
    });
}

#[test]
fn cancelling_one_book_keeps_exporting_the_others() {
    local(async {
        let f = Fixture::new();
        let gate = AsyncGate::default();
        let wait = gate.clone();
        f.converter.on_convert(move |request, cancel| {
            let gate = wait.clone();
            async move {
                if request.metadata.asin == "B1" {
                    gate.wait().await;
                    check(&cancel)?;
                }
                write(&request.output, "")
            }
        });
        let (app, queue) = f.make_app().await;
        export_all(&app);
        wait_until(|| f.converter.requests.current().len() == 1).await;
        let first = app.export().jobs()[0].id;
        app.export().cancel(first);
        gate.open();
        queue.wait_until_finished().await;
        wait_until(|| !app.export().is_running()).await;
        assert_eq!(app.export().jobs()[0].state(), &JobState::Cancelled);
        wait_until(|| asins_of(&app.exported_files()) == ["B2"]).await;
    });
}

#[test]
fn unavailable_folder_is_reported() {
    local(async {
        let f = Fixture::new();
        // A file where the downloads folder should be.
        std::fs::write(&f.downloads.folder, "").unwrap();
        let (app, _) = f.make_app().await;
        app.export().export(&[app.library().books()[0].clone()]);
        let notice = &app.errors().notices()[0];
        assert!(notice.error.technical_error().contains("not writable"));
        assert_eq!(notice.error.title(), "Couldn’t start exports");
        assert!(!app.export().is_running());
    });
}

#[test]
fn sign_in_again_resumes_paused_export() {
    local(async {
        let f = Fixture::new();
        f.api.on_license(|asin, session| async move {
            if session.access_token != "fresh" {
                return Err(ExporterError::AuthenticationRequired);
            }
            Ok(license(&asin))
        });
        f.api
            .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
        f.api
            .on_register(|_, _| async { Ok(session_with_token("fresh")) });
        let (app, queue) = f.make_app().await;
        app.export().export(&[app.library().books()[0].clone()]);

        wait_until(|| app.export().is_waiting_for_authentication()).await;
        wait_until(|| app.session().needs_reauthentication()).await;
        assert_eq!(
            app.export().notice(),
            Some("Paused: sign in to Audible to continue.")
        );
        assert_eq!(app.library().books().len(), 2); // rows stay while re-authenticating

        app.session().sign_in();
        app.session().complete_sign_in(&us_callback()).await;
        wait_until(|| app.export().active_job_count() == 0).await;
        queue.wait_until_finished().await;
        wait_until(|| !app.export().is_running()).await;
        assert_eq!(app.export().notice(), None);
        wait_until(|| asins_of(&app.exported_files()) == ["B1"]).await;
    });
}

#[test]
fn sign_out_clears_library() {
    local(async {
        let f = Fixture::new();
        let (app, _) = f.make_app().await;
        app.library().toggle_selection("B1");
        app.session().sign_out();
        wait_until(|| app.library().books().is_empty()).await;
        assert!(app.library().selection().is_empty());
        assert!(!app.library().can_refresh());
    });
}

#[test]
fn downloaded_books_that_left_the_library_stay_visible() {
    local(async {
        let f = Fixture::new();
        f.keep_download(&titled_book("B1", "Book B1"));
        f.keep_download(&titled_book("GONE", "Returned"));
        let (app, _) = f.make_app().await;

        assert_eq!(library_asins(&app), ["B1", "B2", "GONE"]);
        assert!(app.library().is_in_library("B1"));
        assert!(!app.library().is_in_library("GONE"));
        assert_eq!(app.status(&book("B1")), BookStatus::Downloaded);
        assert_eq!(app.status(&book("B2")), BookStatus::None);
        app.library().set_search_text("returned");
        assert_eq!(app.library().visible_books()[0].asin, "GONE");
        assert_eq!(app.library().visible_books().len(), 1);
    });
}

#[test]
fn rescan_drops_deleted_files_and_their_manifests() {
    local(async {
        let f = Fixture::new();
        f.keep_download(&titled_book("GONE", "Returned"));
        let (app, _) = f.make_app().await;
        assert_eq!(library_asins(&app), ["B1", "B2", "GONE"]);

        std::fs::remove_file(f.downloads.folder.join("Returned [GONE].aaxc")).unwrap();
        app.rescan().await;
        assert_eq!(library_asins(&app), ["B1", "B2"]);
        assert!(f.downloads.manifest("GONE").is_none());
    });
}

#[test]
fn download_from_menu_updates_status_and_menu() {
    local(async {
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
        let (app, queue) = f.make_app().await;
        let target = app.library().books()[0].clone();
        assert_eq!(
            actions(&app.menu_items(&target)),
            [BookAction::Download, BookAction::Export]
        );

        app.perform(BookAction::Download, &target).await;
        wait_until(|| {
            matches!(app.status(&target), BookStatus::Running {
                kind: JobKind::Download, progress, transferred: Some(t)
            } if (progress - 0.5).abs() < 1e-9 && t == kept())
        })
        .await;
        assert!(matches!(
            app.menu_items(&target)[0].action,
            BookAction::Cancel {
                kind: JobKind::Download,
                ..
            }
        ));

        gate.open();
        queue.wait_until_finished().await;
        // Finishing a job triggers a rescan.
        wait_until(|| app.status(&target) == BookStatus::Downloaded).await;
        assert_eq!(
            actions(&app.menu_items(&target)),
            [
                BookAction::Export,
                BookAction::ShowDownload,
                BookAction::DeleteDownload
            ]
        );
    });
}

#[test]
fn selection_buttons_act_on_books_that_offer_the_action() {
    local(async {
        let f = Fixture::new();
        f.keep_download(&titled_book("B1", "Book B1"));
        let (app, queue) = f.make_app().await;
        let books = app.library().books();
        let (downloaded, missing) = (&books[0], &books[1]);
        app.library().set_selection(["B1", "B2"]);
        assert_eq!(
            app.selected_books_offering(BookAction::Download),
            std::slice::from_ref(missing)
        );
        assert_eq!(
            app.selected_books_offering(BookAction::Export),
            [downloaded.clone(), missing.clone()]
        );

        app.perform_on_selection(BookAction::Export);
        assert!(app.library().selection().is_empty());
        queue.wait_until_finished().await;
        wait_until(|| app.status(downloaded) == BookStatus::Exported).await;
        wait_until(|| app.status(missing) == BookStatus::Exported).await;
        // Exporting a book that was not downloaded keeps the download.
        assert!(app.downloaded_files().contains_key("B2"));
    });
}

#[test]
fn download_and_export_run_together() {
    local(async {
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
        let (app, queue) = f.make_app().await;
        let books = app.library().books();
        app.export().download(std::slice::from_ref(&books[0]));
        app.export().export(std::slice::from_ref(&books[1]));
        assert_eq!(app.export().active_job_count(), 2);
        gate.open();
        queue.wait_until_finished().await;
        wait_until(|| !app.export().is_running()).await;
        wait_until(|| app.status(&books[0]) == BookStatus::Downloaded).await;
        wait_until(|| app.status(&books[1]) == BookStatus::Exported).await;
    });
}

fn failed(message: &str) -> BookStatus {
    BookStatus::Failed {
        kind: JobKind::Download,
        message: message.into(),
    }
}

#[test]
fn failure_stays_until_the_next_attempt() {
    local(async {
        let f = Fixture::new();
        f.api.on_license(|asin, _| async move {
            Err(ExporterError::LicenseDenied {
                asin,
                reason: "not owned".into(),
            })
        });
        let (app, queue) = f.make_app().await;
        let books = app.library().books();
        let target = &books[0];
        let denied = failed("The download license was denied: not owned");
        app.export().download(std::slice::from_ref(target));
        queue.wait_until_finished().await;
        wait_until(|| app.status(target) == denied).await;

        // Another book's run drops the finished job from the queue; the failure stays.
        app.export().download(std::slice::from_ref(&books[1]));
        queue.wait_until_finished().await;
        wait_until(|| !app.export().is_running()).await;
        assert_eq!(app.status(target), denied);
        // Download doubles as Retry.
        assert_eq!(
            actions(&app.menu_items(target)),
            [BookAction::Download, BookAction::Export]
        );

        f.license_succeeds();
        app.export().download(std::slice::from_ref(target));
        queue.wait_until_finished().await;
        wait_until(|| app.status(target) == BookStatus::Downloaded).await;
    });
}

#[test]
fn delete_download_keeps_the_export() {
    local(async {
        let f = Fixture::new();
        f.keep_download(&titled_book("B1", "Book B1"));
        std::fs::write(f.downloads.folder.join("Book B1 [B1].m4b"), "m4b").unwrap();
        let (app, _) = f.make_app().await;
        let target = app.library().books()[0].clone();
        assert_eq!(app.status(&target), BookStatus::Exported);

        app.delete_download(&target).await;
        assert!(!app.downloaded_files().contains_key("B1"));
        assert!(app.exported_files().contains_key("B1"));
        assert!(f.downloads.manifest("B1").is_none());
        assert_eq!(
            actions(&app.menu_items(&target)),
            [
                BookAction::Download,
                BookAction::Export,
                BookAction::ShowExport
            ]
        );
    });
}

fn start(app: &AppViewModel, kind: JobKind, target: &Audiobook) {
    match kind {
        JobKind::Download => app.export().download(std::slice::from_ref(target)),
        JobKind::Export => app.export().export(std::slice::from_ref(target)),
    }
}

#[test]
fn license_denial_details_reach_the_toast() {
    for kind in [JobKind::Download, JobKind::Export] {
        local(async {
            let f = Fixture::new();
            let reason =
                "License not granted.\nClient / RequesterEligibility: Client does not have access.";
            f.api.on_license(move |asin, _| async move {
                Err(ExporterError::LicenseDenied {
                    asin,
                    reason: reason.into(),
                })
            });
            let (app, queue) = f.make_app().await;
            let target = app.library().books()[0].clone();
            start(&app, kind, &target);
            queue.wait_until_finished().await;
            wait_until(|| app.errors().notices().len() == 1).await;
            assert_eq!(
                app.errors().notices()[0].error.technical_error(),
                format!(
                    "{}: The download license was denied: {reason}",
                    target.title
                )
            );
        });
    }
}

#[test]
fn job_failure_is_toasted_once_per_attempt() {
    for kind in [JobKind::Download, JobKind::Export] {
        local(async {
            let f = Fixture::new();
            let diagnostic = "Synthetic failure: HTTP 503\nAdditional diagnostic details";
            match kind {
                JobKind::Download => f.downloader.on_download(move |_, _, _| async move {
                    Err(ExporterError::Download(diagnostic.into()))
                }),
                JobKind::Export => f.converter.on_convert(move |_, _| async move {
                    Err(ExporterError::Conversion(diagnostic.into()))
                }),
            }
            let (app, queue) = f.make_app().await;
            let target = app.library().books()[0].clone();
            let title = match kind {
                JobKind::Download => "Download failed",
                JobKind::Export => "Export failed",
            };
            for attempt in 1..=2 {
                start(&app, kind, &target);
                queue.wait_until_finished().await;
                wait_until(|| app.errors().notices().len() == attempt).await;
                let notices = app.errors().notices();
                let last = notices.last().unwrap();
                assert_eq!(last.error.title(), title);
                assert!(last.error.technical_error().contains(diagnostic));
            }
            // More snapshots containing the failed jobs must not reannounce those failures.
            app.export().cancel_all().await;
            assert_eq!(app.errors().notices().len(), 2);
        });
    }
}
