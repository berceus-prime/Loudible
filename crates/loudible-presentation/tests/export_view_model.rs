//! Ported from `ExportViewModelTests.swift` and its extensions.

mod support;

use std::time::Duration;

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

fn failed(message: &str, kept: Option<TransferProgress>) -> BookStatus {
    BookStatus::Failed {
        kind: JobKind::Download,
        message: message.into(),
        kept,
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
        let denied = failed("The download license was denied: not owned", None);
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

#[test]
fn sign_out_pauses_downloads_and_signing_in_again_continues_them() {
    local(async {
        let f = Fixture::new();
        f.downloader
            .on_download(|source, destination, cancel| async move {
                // B2 only ends by being stopped, reporting recovery through the mock.
                if source.path().ends_with("B2.aaxc") {
                    write(&destination, "enc")?;
                    tokio::select! {
                        () = cancel.cancelled() => return Err(ExporterError::Cancelled),
                        () = tokio::time::sleep(Duration::from_secs(60)) => {}
                    }
                }
                write(&destination, "encrypted")
            });
        let (app, queue) = f.make_app_with_two_slots().await;
        app.export().download(&app.library().books());
        wait_until(|| f.downloader.downloads.current().len() == 2).await;
        assert!(app.has_active_jobs());
        app.sign_out().await;
        queue.wait_until_finished().await;
        assert!(!app.has_active_jobs());
        assert!(!app.session().is_signed_in());
        assert_eq!(asins_of(&f.downloads.scan_downloads()), ["B1"]);
        assert_eq!(app.partial_downloads().get("B2"), Some(&kept()));
        assert_eq!(app.partial_downloads().len(), 1);
        wait_until(|| app.library().books().is_empty()).await;

        f.downloader
            .on_download(|_, destination, _| async move { write(&destination, "encrypted") });
        f.api.on_register(|_, _| async { Ok(session()) });
        app.session().sign_in();
        app.session().complete_sign_in(&us_callback()).await;
        wait_until(|| asins_of(&f.downloads.scan_downloads()) == ["B1", "B2"]).await;
        assert_eq!(
            f.downloader.resumes.current().last().unwrap().as_deref(),
            Some(RESUME_TOKEN)
        );
        wait_until(|| app.partial_downloads().is_empty()).await;
    });
}

#[test]
fn launch_continues_the_accounts_downloads_and_drops_the_rest() {
    for has_token in [true, false] {
        local(async {
            let f = Fixture::new();
            let token = has_token.then_some(RESUME_TOKEN);
            f.keep_partial("B1", &session().customer_id, token);
            f.keep_partial("B2", "amzn1.account.SOMEONE.ELSE", Some(RESUME_TOKEN));
            f.keep_partial("B9", &session().customer_id, Some(RESUME_TOKEN)); // no longer in the library
            let (app, queue) = f.make_app().await;
            queue.wait_until_finished().await;
            wait_until(|| asins_of(&app.downloaded_files()) == ["B1"]).await;
            let resumes = f.downloader.resumes.current();
            assert_eq!(resumes.len(), 1);
            assert_eq!(resumes[0].as_deref(), token);
            assert!(f.downloads.partial_downloads().is_empty());
            assert!(app.partial_downloads().is_empty());
            assert_eq!(app.status(&app.library().books()[1]), BookStatus::None);
        });
    }
}

#[test]
fn signed_out_launch_keeps_recovery_and_shows_it_paused() {
    local(async {
        let f = Fixture::new();
        f.store.stored.set(None);
        f.keep_partial("B1", &session().customer_id, Some(RESUME_TOKEN));
        let (app, _) = f.make_app().await;
        assert!(!app.session().is_signed_in());
        assert!(f.downloader.downloads.current().is_empty());
        let progress = TransferProgress::new(3, 9);
        assert_eq!(app.partial_downloads().get("B1"), Some(&progress));
        assert_eq!(app.status(&book("B1")), BookStatus::Paused(progress));
    });
}

#[test]
fn failed_library_load_deletes_nothing() {
    local(async {
        let f = Fixture::new();
        f.api
            .on_library(|_| async { Err(ExporterError::Network("offline".into())) });
        f.keep_partial("B9", &session().customer_id, Some(RESUME_TOKEN));
        let (app, _) = f.make_app().await;
        assert!(app.library().error_message().is_some());
        assert!(f.downloads.partial_download("B9").is_some());
        assert!(f.downloader.downloads.current().is_empty());
    });
}

fn fails_with_http_500(f: &Fixture) {
    f.downloader.on_download(|_, destination, _| async move {
        write(&destination, "enc")?;
        Err(ExporterError::Download("HTTP 500".into()))
    });
}

#[test]
fn failed_download_keeps_recovery_until_resumed_or_discarded() {
    local(async {
        let f = Fixture::new();
        fails_with_http_500(&f);
        let (app, queue) = f.make_app().await;
        let target = app.library().books()[0].clone();
        app.export().download(std::slice::from_ref(&target));
        queue.wait_until_finished().await;
        let failure = failed("Download failed: HTTP 500", Some(kept()));
        wait_until(|| app.status(&target) == failure).await;
        assert_eq!(
            actions(&app.menu_items(&target)),
            [
                BookAction::ResumeDownload,
                BookAction::Export,
                BookAction::DeleteDownload
            ]
        );
        assert!(app.selected_books_offering(BookAction::Download).is_empty());
        app.library().toggle_selection(&target.asin);
        assert_eq!(
            app.selected_books_offering(BookAction::Download),
            std::slice::from_ref(&target)
        );

        // A refresh doesn't retry it by itself.
        app.refresh_library().await;
        assert_eq!(f.downloader.downloads.current().len(), 1);

        app.delete_download(&target).await;
        assert_eq!(
            app.status(&target),
            failed("Download failed: HTTP 500", None)
        );
        assert!(f.downloads.partial_downloads().is_empty());
        assert_eq!(
            actions(&app.menu_items(&target)),
            [BookAction::Download, BookAction::Export]
        );
    });
}

#[test]
fn resume_download_reuses_the_token() {
    local(async {
        let f = Fixture::new();
        fails_with_http_500(&f);
        let (app, queue) = f.make_app().await;
        let target = app.library().books()[0].clone();
        app.perform(BookAction::Download, &target).await;
        queue.wait_until_finished().await;
        wait_until(|| app.partial_downloads().contains_key(&target.asin)).await;

        f.downloader
            .on_download(|_, destination, _| async move { write(&destination, "encrypted") });
        app.perform(BookAction::ResumeDownload, &target).await;
        queue.wait_until_finished().await;
        wait_until(|| app.status(&target) == BookStatus::Downloaded).await;
        assert_eq!(
            f.downloader.resumes.current().last().unwrap().as_deref(),
            Some(RESUME_TOKEN)
        );
        assert!(app.partial_downloads().is_empty());
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
