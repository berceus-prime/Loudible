use BookAction as A;
use loudible_core::*;
use loudible_test_support::*;

fn job(kind: JobKind, states: Vec<JobState>, asin: &str) -> ExportJob {
    let mut job = ExportJob::new(book(asin), kind);
    for state in states {
        job.transition(state).unwrap();
    }
    job
}

fn running(phase: Phase, progress: f64) -> JobState {
    JobState::Running(phase, progress)
}

fn resolve(
    jobs: &[ExportJob],
    failure: Option<(JobKind, &str)>,
    downloaded: bool,
    exported: bool,
) -> BookStatus {
    BookStatus::resolve("B1", jobs, failure, downloaded, exported)
}

const X: Option<(JobKind, &str)> = Some((JobKind::Download, "x"));

#[test]
fn priority_running_queued_failed_exported_downloaded() {
    let run = job(JobKind::Export, vec![running(Phase::Converting, 0.0)], "B1");
    let queued = job(JobKind::Download, vec![], "B1");
    assert_eq!(
        resolve(&[queued.clone(), run], X, true, true),
        BookStatus::Running {
            kind: JobKind::Export,
            progress: 0.0,
            transferred: None
        }
    );
    assert_eq!(
        resolve(&[queued], X, true, true),
        BookStatus::Queued(JobKind::Download)
    );
    assert_eq!(
        resolve(&[], X, true, true),
        BookStatus::Failed {
            kind: JobKind::Download,
            message: "x".into(),
        }
    );
    assert_eq!(resolve(&[], None, true, true), BookStatus::Exported);
    assert_eq!(resolve(&[], None, true, false), BookStatus::Downloaded);
    assert_eq!(resolve(&[], None, false, false), BookStatus::None);
}

#[test]
fn ignores_finished_jobs_and_other_books() {
    let finished = job(
        JobKind::Download,
        vec![running(Phase::Licensing, 0.0), JobState::Failed("x".into())],
        "B1",
    );
    let other = job(
        JobKind::Download,
        vec![running(Phase::Licensing, 0.0)],
        "B2",
    );
    assert_eq!(
        resolve(&[finished, other], None, true, false),
        BookStatus::Downloaded
    );
}

#[test]
fn paused_for_sign_in_shows_as_queued() {
    let waiting = job(
        JobKind::Download,
        vec![JobState::WaitingForAuthentication],
        "B1",
    );
    assert_eq!(
        resolve(&[waiting], None, false, false),
        BookStatus::Queued(JobKind::Download)
    );
}

fn context() -> BookMenuContext {
    BookMenuContext {
        is_in_library: true,
        has_manifest: true,
        is_signed_in: true,
        ..Default::default()
    }
}

fn actions(jobs: &[ExportJob], context: BookMenuContext) -> Vec<BookAction> {
    book_menu_items("B1", jobs, &context)
        .into_iter()
        .map(|item| item.action)
        .collect()
}

fn export(context: BookMenuContext) -> BookMenuItem {
    book_menu_items("B1", &[], &context)
        .into_iter()
        .find(|item| item.action == A::Export)
        .unwrap()
}

#[test]
fn not_downloaded_offers_download_and_export() {
    assert_eq!(actions(&[], context()), [A::Download, A::Export]);
    // Like Download, it fetches the keys when the job runs.
    assert!(
        export(BookMenuContext {
            has_manifest: false,
            is_signed_in: false,
            ..context()
        })
        .is_enabled
    );
    // There is nothing to download for a book that left the library.
    assert_eq!(
        actions(
            &[],
            BookMenuContext {
                is_in_library: false,
                ..context()
            }
        ),
        []
    );
}

#[test]
fn running_or_queued_jobs_offer_cancel_only() {
    let download = job(
        JobKind::Download,
        vec![running(Phase::Downloading, 0.3)],
        "B1",
    );
    let queued = job(JobKind::Export, vec![], "B1");
    let expected = [
        A::Cancel {
            job_id: download.id,
            kind: JobKind::Download,
        },
        A::Cancel {
            job_id: queued.id,
            kind: JobKind::Export,
        },
    ];
    assert_eq!(actions(&[download, queued], context()), expected);
}

#[test]
fn downloaded_offers_export_finder_and_delete() {
    let downloaded = BookMenuContext {
        is_downloaded: true,
        ..context()
    };
    assert_eq!(
        actions(&[], downloaded),
        [A::Export, A::ShowDownload, A::DeleteDownload]
    );
    assert_eq!(
        actions(
            &[],
            BookMenuContext {
                is_exported: true,
                ..downloaded
            }
        ),
        [A::Export, A::ShowDownload, A::ShowExport, A::DeleteDownload]
    );
    assert!(export(downloaded).is_enabled);
}

#[test]
fn exported_without_download_offers_download_export_and_show_export() {
    assert_eq!(
        actions(
            &[],
            BookMenuContext {
                is_exported: true,
                ..context()
            }
        ),
        [A::Download, A::Export, A::ShowExport]
    );
}

#[test]
fn busy_downloaded_book_disables_export_and_hides_delete() {
    let run = job(JobKind::Export, vec![running(Phase::Converting, 0.1)], "B1");
    let cancel = A::Cancel {
        job_id: run.id,
        kind: JobKind::Export,
    };
    let items = book_menu_items(
        "B1",
        &[run],
        &BookMenuContext {
            is_downloaded: true,
            ..context()
        },
    );
    let kinds: Vec<BookAction> = items.iter().map(|item| item.action).collect();
    assert_eq!(kinds, [cancel, A::Export, A::ShowDownload]);
    assert!(!items[1].is_enabled);
    assert!(items[1].disabled_reason.is_some());
}

#[test]
fn export_is_disabled_with_a_reason() {
    let downloaded = BookMenuContext {
        is_downloaded: true,
        ..context()
    };
    let not_in_library = export(BookMenuContext {
        is_in_library: false,
        ..downloaded
    });
    assert!(!not_in_library.is_enabled);
    assert_eq!(
        not_in_library.disabled_reason.as_deref(),
        Some("This book is no longer in your library.")
    );

    let no_keys = export(BookMenuContext {
        has_manifest: false,
        is_signed_in: false,
        ..downloaded
    });
    assert!(!no_keys.is_enabled);
    assert!(no_keys.disabled_reason.unwrap().contains("Sign in"));

    // Stored keys make sign-in unnecessary; a missing manifest is fine while signed in.
    assert!(
        export(BookMenuContext {
            is_signed_in: false,
            ..downloaded
        })
        .is_enabled
    );
    assert!(
        export(BookMenuContext {
            has_manifest: false,
            ..downloaded
        })
        .is_enabled
    );
}
