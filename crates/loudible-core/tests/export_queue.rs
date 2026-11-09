use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use loudible_core::JobState::*;
use loudible_core::export_file_naming::is_partial;
use loudible_core::export_queue::book_export_workflow::{BookExportWorkflow, REDOWNLOAD_HINT};
use loudible_core::export_queue::download_workflow::DownloadWorkflow;
use loudible_core::export_queue::execution_context::{ExportExecutionContext, ExportProgressEvent};
use loudible_core::export_queue::file_finalizer::ExportFileFinalizer;
use loudible_core::export_queue::license_provider::LicenseProvider;
use loudible_core::export_queue::state::ExportQueueState;
use loudible_core::*;
use loudible_test_support::*;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const B1: &str = "B000000001";
const B2: &str = "B000000002";
const B3: &str = "B000000003";

struct Fixture {
    api: Arc<MockAudibleApi>,
    store: Arc<InMemoryCredentialStore>,
    downloader: Arc<MockDownloader>,
    converter: Arc<MockConverter>,
    destination: TemporaryDirectory,
    work: TemporaryDirectory,
    downloads: LocalDownloads,
}

impl Fixture {
    fn new() -> Self {
        let work = TemporaryDirectory::default();
        let downloads = LocalDownloads::with_trash(
            work.join("Downloads"),
            work.join("Manifests"),
            Arc::new(|path: &Path| std::fs::remove_file(path)),
        );
        let api = Arc::new(MockAudibleApi::default());
        api.on_license(|asin, _| async move { Ok(license(&asin)) });
        Self {
            api,
            store: Arc::new(InMemoryCredentialStore::with(session())),
            downloader: Arc::default(),
            converter: Arc::default(),
            destination: TemporaryDirectory::default(),
            work,
            downloads,
        }
    }

    fn auth(&self) -> AuthenticationCoordinator {
        let auth = AuthenticationCoordinator::new(
            self.api.clone(),
            self.store.clone(),
            Arc::new(TestClock::default()),
        );
        auth.restore().unwrap();
        auth
    }

    /// One book at a time unless a test is about concurrency, so the order is deterministic.
    fn queue(&self, maximum: usize) -> (ExportQueue, AuthenticationCoordinator) {
        let auth = self.auth();
        (
            self.queue_with(auth.clone(), self.downloads.clone(), maximum),
            auth,
        )
    }

    fn queue_with(
        &self,
        auth: AuthenticationCoordinator,
        downloads: LocalDownloads,
        maximum: usize,
    ) -> ExportQueue {
        ExportQueue::new(
            auth,
            self.api.clone(),
            self.downloader.clone(),
            self.converter.clone(),
            downloads,
            maximum,
        )
    }

    async fn run(&self, books: &[Audiobook], kind: JobKind) -> Snapshot {
        let (queue, _) = self.queue(1);
        queue
            .enqueue(books, kind, Some(self.destination.path()))
            .unwrap();
        queue.wait_until_finished().await;
        queue.current_snapshot()
    }

    fn downloads_contents(&self) -> Vec<String> {
        contents_of(&self.downloads.folder)
    }

    fn manifest_contents(&self) -> Vec<String> {
        contents_of(&self.downloads.manifest_folder)
    }

    /// A kept download as an earlier run would have left it.
    fn keep(&self, book: &Audiobook, name: Option<&str>, with_manifest: bool) -> PathBuf {
        self.downloads.ensure_folder().unwrap();
        let default = format!("{} [{}].aaxc", book.title, book.asin);
        let path = self.downloads.folder.join(name.unwrap_or(&default));
        std::fs::write(&path, "kept").unwrap();
        if with_manifest {
            self.downloads
                .write_manifest(
                    &DownloadManifest::new(
                        book.clone(),
                        stored_keys(),
                        vec![Chapter {
                            title: "Stored".into(),
                            start_milliseconds: 0,
                            length_milliseconds: 1,
                        }],
                        reference_date(),
                    ),
                    None,
                )
                .unwrap();
        }
        path
    }

    fn workflow(&self) -> DownloadWorkflow {
        self.downloads.ensure_folder().unwrap();
        DownloadWorkflow {
            licenses: LicenseProvider {
                auth: self.auth(),
                api: self.api.clone(),
            },
            downloader: self.downloader.clone(),
            downloads: self.downloads.clone(),
            finalizer: ExportFileFinalizer,
        }
    }

    fn exporter(&self) -> BookExportWorkflow {
        BookExportWorkflow {
            download: self.workflow(),
            converter: self.converter.clone(),
            finalizer: ExportFileFinalizer,
        }
    }

    fn output(&self, name: &str) -> JobState {
        Completed(self.destination.join(name))
    }

    fn fail_refresh_until_signed_in(&self) {
        self.api.on_license(|asin, session| async move {
            if session.access_token != "fresh" {
                return Err(ExporterError::AuthenticationRequired);
            }
            Ok(license(&asin))
        });
        self.api
            .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
        self.api
            .on_register(|_, _| async { Ok(session_with_token("fresh")) });
    }

    fn reject_everything(&self) {
        self.api
            .on_license(|_, _| async { Err(ExporterError::AuthenticationRequired) });
        self.api
            .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
    }

    /// Converting `asin` writes a partial file, signals, waits at the gate, then checks for
    /// cancellation. Other books convert immediately.
    fn hold_conversion(&self, asin: &'static str) -> (AsyncGate, Arc<Locked<bool>>) {
        let gate = AsyncGate::default();
        let started = Arc::new(Locked::new(false));
        let (held, mark) = (gate.clone(), started.clone());
        self.converter.on_convert(move |request, cancel| {
            let (held, mark) = (held.clone(), mark.clone());
            async move {
                if request.metadata.asin == asin {
                    write(&request.output, "partial")?;
                    mark.set(true);
                    held.wait().await;
                    check(&cancel)?;
                }
                write(&request.output, "m4b")
            }
        });
        (gate, started)
    }

    fn hold_downloads(&self) -> AsyncGate {
        let gate = AsyncGate::default();
        let held = gate.clone();
        self.downloader.on_download(move |_, destination, _| {
            let held = held.clone();
            async move {
                held.wait().await;
                write(&destination, "")
            }
        });
        gate
    }

    /// Downloads write a partial file, wait at the gate, then end as cancelled.
    fn interrupt_downloads(&self) -> AsyncGate {
        let gate = AsyncGate::default();
        let held = gate.clone();
        self.downloader.on_download(move |_, destination, _| {
            let held = held.clone();
            async move {
                write(&destination, "partial")?;
                held.wait().await;
                Err(ExporterError::Cancelled)
            }
        });
        gate
    }

    fn fail_downloads(&self) {
        self.downloader.on_download(|_, destination, _| async move {
            write(&destination, "partial")?;
            Err(ExporterError::Download("HTTP 500".into()))
        });
    }

    async fn wait_for_downloads(&self, count: usize) {
        let downloader = self.downloader.clone();
        wait_until(move || downloader.downloads.current().len() == count).await;
    }
}

fn stored_keys() -> Decryption {
    Decryption::Aax {
        activation_bytes: vec![1, 2, 3, 4],
    }
}

fn books(count: usize) -> Vec<Audiobook> {
    (1..=count)
        .map(|n| {
            book_in(
                &format!("B00000000{n}"),
                &format!("Book {n}"),
                Marketplace::US,
            )
        })
        .collect()
}

fn context(keep_recovery: bool, cancel: CancellationToken) -> ExportExecutionContext {
    ExportExecutionContext {
        report: Arc::new(|_| {}),
        keeps_recovery: Arc::new(move || keep_recovery),
        cancel,
    }
}

fn plain_context() -> ExportExecutionContext {
    context(false, CancellationToken::new())
}

fn states(snapshot: &Snapshot) -> Vec<JobState> {
    snapshot
        .jobs
        .iter()
        .map(|job| job.state().clone())
        .collect()
}

fn all_completed(snapshot: &Snapshot) -> bool {
    snapshot
        .jobs
        .iter()
        .all(|job| matches!(job.state(), Completed(_)))
}

async fn wait_for_pause(queue: &ExportQueue) {
    let queue = queue.clone();
    wait_until(move || queue.current_snapshot().is_waiting_for_authentication).await;
}

// MARK: Book export workflow

async fn kept_file_uses_manifest_or_fresh_license(with_manifest: bool) {
    let f = Fixture::new();
    let book = &books(1)[0];
    let kept = f.keep(book, None, with_manifest);
    let output = f
        .exporter()
        .run(book, f.destination.path(), &plain_context())
        .await
        .unwrap();
    let request = f.converter.requests.current().remove(0);
    assert_eq!(request.input, kept);
    assert_eq!(request.metadata.asin, book.asin);
    let expected = if with_manifest {
        stored_keys()
    } else {
        license(&book.asin).decryption
    };
    assert_eq!(request.decryption, expected);
    assert_eq!(
        f.api.requested_licenses.current().len(),
        usize::from(!with_manifest)
    );
    assert!(f.downloader.downloads.current().is_empty());
    assert!(output.exists());
}

#[tokio::test]
async fn kept_file_uses_manifest() {
    kept_file_uses_manifest_or_fresh_license(true).await;
}

#[tokio::test]
async fn kept_file_without_manifest_uses_fresh_license() {
    kept_file_uses_manifest_or_fresh_license(false).await;
}

#[tokio::test]
async fn fresh_download_shares_license_with_conversion() {
    let f = Fixture::new();
    let book = &books(1)[0];
    f.exporter()
        .run(book, f.destination.path(), &plain_context())
        .await
        .unwrap();
    assert_eq!(
        f.api.requested_licenses.current(),
        std::slice::from_ref(&book.asin)
    );
    assert_eq!(
        f.converter.requests.current()[0].decryption,
        license(&book.asin).decryption
    );
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aaxc"]);
}

#[tokio::test]
async fn failed_conversion_cleans_output_and_suggests_redownload() {
    let f = Fixture::new();
    let book = &books(1)[0];
    f.keep(book, None, true);
    f.converter.on_convert(|request, _| async move {
        write(&request.output, "partial")?;
        Err(ExporterError::Conversion("bad".into()))
    });
    let result = f
        .exporter()
        .run(book, f.destination.path(), &plain_context())
        .await;
    assert_eq!(
        result,
        Err(ExporterError::Conversion(format!("bad {REDOWNLOAD_HINT}")))
    );
    assert!(f.destination.contents().is_empty());
    assert!(f.downloads.scan_downloads().contains_key(&book.asin));
}

#[tokio::test]
async fn cancellation_during_finalizing_removes_output_and_keeps_download() {
    let f = Fixture::new();
    let book = &books(1)[0];
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let context = ExportExecutionContext {
        report: Arc::new(move |event| {
            if matches!(event, ExportProgressEvent::Phase(Phase::Finalizing)) {
                trigger.cancel();
            }
        }),
        keeps_recovery: Arc::new(|| false),
        cancel,
    };
    assert_eq!(
        f.exporter().run(book, f.destination.path(), &context).await,
        Err(ExporterError::Cancelled)
    );
    assert!(f.destination.contents().is_empty());
    assert!(f.downloads.scan_downloads().contains_key(&book.asin));
}

#[tokio::test]
async fn missing_output_fails() {
    let f = Fixture::new();
    f.converter.on_convert(|_, _| async { Ok(()) });
    let result = f
        .exporter()
        .run(&books(1)[0], f.destination.path(), &plain_context())
        .await;
    assert_eq!(
        result,
        Err(ExporterError::Conversion(
            "The converter did not produce an output file.".into()
        ))
    );
}

// MARK: Download workflow

#[tokio::test]
async fn direct_download_persists_manifest_and_reuses_file() {
    let f = Fixture::new();
    let workflow = f.workflow();
    let book = &books(1)[0];
    let result = workflow
        .run(book, JobKind::Download, &plain_context())
        .await
        .unwrap();
    assert!(result.fresh_license.is_some());
    assert_eq!(f.downloads.manifest(&book.asin).unwrap().book, *book);
    let again = workflow
        .run(book, JobKind::Download, &plain_context())
        .await
        .unwrap();
    assert_eq!(again.input, result.input);
    assert!(again.fresh_license.is_none());
    assert_eq!(
        f.api.requested_licenses.current(),
        std::slice::from_ref(&book.asin)
    );
    assert!(f.downloads.partial_downloads().is_empty());
}

async fn cancellation_applies_recovery_policy(keep: bool) {
    let f = Fixture::new();
    let workflow = f.workflow();
    let started = AsyncGate::default();
    let cleanup = AsyncGate::default();
    let (open, wait) = (started.clone(), cleanup.clone());
    f.downloader.on_download(move |_, _, cancel| {
        let (open, wait) = (open.clone(), wait.clone());
        async move {
            open.open();
            wait.wait().await;
            check(&cancel)
        }
    });
    let cancel = CancellationToken::new();
    let run = {
        let context = context(keep, cancel.clone());
        let book = books(1).remove(0);
        tokio::spawn(async move {
            workflow
                .run(&book, JobKind::Download, &context)
                .await
                .map(|_| ())
        })
    };
    started.wait().await;
    cancel.cancel();
    cleanup.open();
    let _ = run.await.unwrap();
    assert_eq!(f.downloads.partial_download(B1).is_some(), keep);
    assert!(f.downloads_contents().is_empty());
}

#[tokio::test]
async fn cancellation_discards_recovery() {
    cancellation_applies_recovery_policy(false).await;
}

#[tokio::test]
async fn suspension_keeps_recovery() {
    cancellation_applies_recovery_policy(true).await;
}

#[tokio::test]
async fn failure_keeps_recovery_and_next_run_resumes() {
    let f = Fixture::new();
    let workflow = f.workflow();
    let book = &books(1)[0];
    f.downloader
        .on_download(|_, _, _| async { Err(ExporterError::Download("failed".into())) });
    let result = workflow
        .run(book, JobKind::Download, &plain_context())
        .await;
    assert_eq!(result.err(), Some(ExporterError::Download("failed".into())));
    f.downloader
        .on_download(|_, destination, _| async move { write(&destination, "audio") });
    workflow
        .run(book, JobKind::Download, &plain_context())
        .await
        .unwrap();
    assert_eq!(
        f.downloader.resumes.current().last().unwrap().as_deref(),
        Some(RESUME_TOKEN)
    );
}

// MARK: File finalizer

#[test]
fn collisions_preserve_existing_files() {
    let directory = TemporaryDirectory::default();
    let partial = directory.join(".output.partial");
    let existing = directory.join("Book.m4b");
    std::fs::write(&existing, "old").unwrap();
    std::fs::write(&partial, "new").unwrap();
    let output = ExportFileFinalizer
        .move_into_place(&partial, directory.path(), "Book", "m4b")
        .unwrap();
    assert_eq!(output.file_name().unwrap(), "Book (2).m4b");
    assert_eq!(std::fs::read(existing).unwrap(), b"old");
    assert_eq!(std::fs::read(output).unwrap(), b"new");
    assert!(!partial.exists());
}

#[test]
fn missing_input_or_directory_fails() {
    let directory = TemporaryDirectory::default();
    let input = directory.join("input");
    assert!(
        ExportFileFinalizer
            .move_into_place(&input, directory.path(), "Book", "m4b")
            .is_err()
    );
    std::fs::write(&input, "").unwrap();
    assert!(
        ExportFileFinalizer
            .move_into_place(&input, &directory.join("missing"), "Book", "m4b")
            .is_err()
    );
    // A failed move leaves the finished file alone.
    assert!(input.exists());
}

// MARK: Authentication

#[tokio::test]
async fn pauses_for_authentication_and_resumes_after_sign_in() {
    let f = Fixture::new();
    f.fail_refresh_until_signed_in();
    let (queue, auth) = f.queue(1);
    queue.start(&books(2), f.destination.path()).unwrap();
    wait_for_pause(&queue).await;
    assert_eq!(
        states(&queue.current_snapshot()),
        [WaitingForAuthentication, Queued]
    );
    assert_eq!(
        auth.state(),
        AuthState::ReauthenticationRequired(Marketplace::US)
    );

    // The user signs in again; the coordinator now holds a working session.
    auth.begin_login(Marketplace::US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    queue.resume_after_authentication();
    queue.wait_until_finished().await;
    let snapshot = queue.current_snapshot();
    assert!(all_completed(&snapshot));
    assert!(!snapshot.is_waiting_for_authentication);
}

#[tokio::test]
async fn several_jobs_pause_for_authentication_and_all_resume() {
    let f = Fixture::new();
    f.fail_refresh_until_signed_in();
    let (queue, auth) = f.queue(3);
    queue.enqueue(&books(2), JobKind::Download, None).unwrap();
    let watched = queue.clone();
    wait_until(move || {
        states(&watched.current_snapshot())
            .iter()
            .all(|s| *s == WaitingForAuthentication)
    })
    .await;
    assert!(queue.current_snapshot().is_waiting_for_authentication);

    auth.begin_login(Marketplace::US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    queue.resume_after_authentication();
    queue.wait_until_finished().await;
    assert!(all_completed(&queue.current_snapshot()));
    assert_eq!(f.downloads_contents().len(), 2);
}

#[tokio::test]
async fn paused_export_of_kept_download_resumes() {
    let f = Fixture::new();
    // No manifest, so the export needs a license and pauses for sign-in.
    f.keep(&books(1)[0], None, false);
    f.fail_refresh_until_signed_in();
    let (queue, auth) = f.queue(1);
    queue.start(&books(1), f.destination.path()).unwrap();
    wait_for_pause(&queue).await;
    auth.begin_login(Marketplace::US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    queue.resume_after_authentication();
    queue.wait_until_finished().await;
    assert!(f.downloader.downloads.current().is_empty());
    assert_eq!(
        states(&queue.current_snapshot()),
        [f.output("Book 1 [B000000001].m4b")]
    );
}

#[tokio::test]
async fn cancel_while_waiting_for_authentication() {
    let f = Fixture::new();
    f.reject_everything();
    let (queue, _) = f.queue(1);
    queue.start(&books(2), f.destination.path()).unwrap();
    wait_for_pause(&queue).await;
    queue.cancel_all();
    queue.wait_until_finished().await;
    assert_eq!(states(&queue.current_snapshot()), [Cancelled, Cancelled]);
    assert!(!queue.current_snapshot().is_running);
}

#[tokio::test]
async fn resuming_while_still_signed_out_waits_again() {
    let f = Fixture::new();
    f.reject_everything();
    let (queue, _) = f.queue(1);
    queue.start(&books(1), f.destination.path()).unwrap();
    wait_for_pause(&queue).await;
    queue.resume_after_authentication();
    let watched = queue.clone();
    wait_until(move || states(&watched.current_snapshot()) == [WaitingForAuthentication]).await;
    wait_for_pause(&queue).await;
    queue.cancel_all();
    queue.wait_until_finished().await;
    assert_eq!(states(&queue.current_snapshot()), [Cancelled]);
}

// MARK: Control

async fn wait_for(flag: &Arc<Locked<bool>>) {
    let flag = flag.clone();
    wait_until(move || flag.current()).await;
}

#[tokio::test]
async fn cancel_stops_current_and_queued_books_but_keeps_completed() {
    let f = Fixture::new();
    let (gate, started) = f.hold_conversion(B2);
    let (queue, _) = f.queue(1);
    queue.start(&books(3), f.destination.path()).unwrap();
    wait_for(&started).await;
    queue.cancel_all();
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(
        states(&queue.current_snapshot()),
        [f.output("Book 1 [B000000001].m4b"), Cancelled, Cancelled]
    );
    assert_eq!(f.destination.contents(), ["Book 1 [B000000001].m4b"]);
    // Finished downloads are kept, including the one whose conversion was cancelled.
    assert_eq!(
        f.downloads_contents(),
        ["Book 1 [B000000001].aaxc", "Book 2 [B000000002].aaxc"]
    );
    assert_eq!(f.api.requested_licenses.current(), [B1, B2]);
}

async fn cancel_during_download_removes_the_partial(kind: JobKind) {
    let f = Fixture::new();
    let gate = f.interrupt_downloads();
    let (queue, _) = f.queue(1);
    queue
        .enqueue(&books(1), kind, Some(f.destination.path()))
        .unwrap();
    f.wait_for_downloads(1).await;
    queue.cancel_all();
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(states(&queue.current_snapshot()), [Cancelled]);
    assert!(f.downloads_contents().is_empty());
    assert!(f.manifest_contents().is_empty());
    assert!(f.destination.contents().is_empty());
}

#[tokio::test]
async fn cancel_during_download_job_removes_the_partial() {
    cancel_during_download_removes_the_partial(JobKind::Download).await;
}

#[tokio::test]
async fn cancel_during_export_job_removes_the_partial() {
    cancel_during_download_removes_the_partial(JobKind::Export).await;
}

#[tokio::test]
async fn cancelling_running_job_continues_with_the_rest() {
    let f = Fixture::new();
    let (gate, started) = f.hold_conversion(B2);
    let (queue, _) = f.queue(1);
    queue.start(&books(3), f.destination.path()).unwrap();
    wait_for(&started).await;
    queue.cancel(queue.current_snapshot().jobs[1].id);
    gate.open();
    queue.wait_until_finished().await;
    let snapshot = queue.current_snapshot();
    assert_eq!(
        states(&snapshot),
        [
            f.output("Book 1 [B000000001].m4b"),
            Cancelled,
            f.output("Book 3 [B000000003].m4b")
        ]
    );
    assert_eq!(
        f.destination.contents(),
        ["Book 1 [B000000001].m4b", "Book 3 [B000000003].m4b"]
    );
    assert!(!f.downloads_contents().iter().any(|name| is_partial(name)));
    assert!(!snapshot.is_running);
}

#[tokio::test]
async fn cancelling_queued_job_skips_it() {
    let f = Fixture::new();
    let (gate, started) = f.hold_conversion(B1);
    let (queue, _) = f.queue(1);
    queue.start(&books(3), f.destination.path()).unwrap();
    wait_for(&started).await;
    queue.cancel(queue.current_snapshot().jobs[1].id);
    assert_eq!(states(&queue.current_snapshot())[1], Cancelled);
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(
        states(&queue.current_snapshot()),
        [
            f.output("Book 1 [B000000001].m4b"),
            Cancelled,
            f.output("Book 3 [B000000003].m4b")
        ]
    );
    assert_eq!(f.api.requested_licenses.current(), [B1, B3]);
}

#[tokio::test]
async fn cancelling_finished_or_unknown_job_changes_nothing() {
    let f = Fixture::new();
    let (queue, _) = f.queue(1);
    queue.start(&books(1), f.destination.path()).unwrap();
    queue.wait_until_finished().await;
    let before = queue.current_snapshot();
    queue.cancel(before.jobs[0].id);
    queue.cancel(Uuid::new_v4());
    assert_eq!(queue.current_snapshot(), before);
}

#[tokio::test]
async fn clear_forgets_finished_jobs_but_not_running_ones() {
    let f = Fixture::new();
    let gate = AsyncGate::default();
    let held = gate.clone();
    f.converter.on_convert(move |request, _| {
        let held = held.clone();
        async move {
            held.wait().await;
            write(&request.output, "m4b")
        }
    });
    let (queue, _) = f.queue(1);
    queue.start(&books(1), f.destination.path()).unwrap();
    queue.clear();
    assert_eq!(queue.current_snapshot().jobs.len(), 1);
    gate.open();
    queue.wait_until_finished().await;
    queue.clear();
    assert_eq!(queue.current_snapshot(), Snapshot::default());
    assert_eq!(f.destination.contents(), ["Book 1 [B000000001].m4b"]);
}

// MARK: Download integration

#[tokio::test]
async fn download_job_keeps_file_and_writes_manifest() {
    let f = Fixture::new();
    let snapshot = f.run(&books(1), JobKind::Download).await;
    assert_eq!(
        states(&snapshot),
        [Completed(
            f.downloads.folder.join("Book 1 [B000000001].aaxc")
        )]
    );
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aaxc"]);
    assert!(f.converter.requests.current().is_empty());
    assert!(f.destination.contents().is_empty());

    let manifest = f.downloads.manifest(B1).unwrap();
    let expected = license(B1);
    assert_eq!(manifest.decryption, expected.decryption);
    assert_eq!(manifest.chapters, expected.chapters);
    assert_eq!(manifest.book, books(1)[0]);
    assert_eq!(manifest.file_extension, "aaxc");
}

#[tokio::test]
async fn legacy_aax_download_gets_aax_extension() {
    let f = Fixture::new();
    f.api.on_license(|asin, _| async move {
        Ok(ContentLicense {
            decryption: stored_keys(),
            chapters: vec![],
            ..license(&asin)
        })
    });
    f.run(&books(1), JobKind::Download).await;
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aax"]);
}

#[tokio::test]
async fn already_downloaded_book_completes_without_license_or_download() {
    let f = Fixture::new();
    let kept = f.keep(
        &books(1)[0],
        Some("Renamed by user [B000000001].aaxc"),
        true,
    );
    let snapshot = f.run(&books(1), JobKind::Download).await;
    assert_eq!(states(&snapshot), [Completed(kept)]);
    assert!(f.api.requested_licenses.current().is_empty());
    assert!(f.downloader.downloads.current().is_empty());
}

#[tokio::test]
async fn export_of_downloaded_book_uses_local_file_and_stored_keys() {
    let f = Fixture::new();
    let kept = f.keep(&books(1)[0], None, true);
    let snapshot = f.run(&books(1), JobKind::Export).await;
    assert!(f.api.requested_licenses.current().is_empty());
    assert!(f.downloader.downloads.current().is_empty());
    let request = f.converter.requests.current().remove(0);
    assert_eq!(request.input, kept);
    assert_eq!(request.decryption, stored_keys());
    assert_eq!(request.chapters[0].title, "Stored");
    assert_eq!(states(&snapshot), [f.output("Book 1 [B000000001].m4b")]);
    assert_eq!(std::fs::read(kept).unwrap(), b"kept"); // the download is never deleted
}

#[tokio::test]
async fn export_of_downloaded_book_without_manifest_gets_fresh_keys_but_does_not_download() {
    let f = Fixture::new();
    let kept = f.keep(&books(1)[0], None, false);
    f.run(&books(1), JobKind::Export).await;
    assert_eq!(f.api.requested_licenses.current(), [B1]);
    assert!(f.downloader.downloads.current().is_empty());
    let request = f.converter.requests.current().remove(0);
    assert_eq!(request.input, kept);
    assert_eq!(request.decryption, license(B1).decryption);
}

#[tokio::test]
async fn failed_conversion_of_kept_download_suggests_downloading_again() {
    let f = Fixture::new();
    f.keep(&books(1)[0], None, true);
    f.converter
        .on_convert(|_, _| async { Err(ExporterError::Conversion("bad packet".into())) });
    let snapshot = f.run(&books(1), JobKind::Export).await;
    let Failed(message) = snapshot.jobs[0].state() else {
        panic!("not failed");
    };
    assert!(message.ends_with("If this keeps failing, delete the download and download it again."));
}

#[tokio::test]
async fn download_then_export_of_one_book_run_in_order_and_share_the_file() {
    let f = Fixture::new();
    let events = Arc::new(Locked::new(Vec::<String>::new()));
    let gate = AsyncGate::default();
    let (record, held) = (events.clone(), gate.clone());
    f.downloader.on_download(move |_, destination, _| {
        let (record, held) = (record.clone(), held.clone());
        async move {
            record.with(|all| all.push("download start".into()));
            held.wait().await;
            write(&destination, "enc")?;
            record.with(|all| all.push("download end".into()));
            Ok(())
        }
    });
    let record = events.clone();
    f.converter.on_convert(move |request, _| {
        let record = record.clone();
        async move {
            record.with(|all| all.push("convert".into()));
            write(&request.output, "m4b")
        }
    });
    let (queue, _) = f.queue(3);
    queue.enqueue(&books(1), JobKind::Download, None).unwrap();
    queue
        .enqueue(&books(1), JobKind::Export, Some(f.destination.path()))
        .unwrap();
    let watched = events.clone();
    wait_until(move || watched.current() == ["download start"]).await;
    assert_eq!(states(&queue.current_snapshot())[1], Queued);
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(
        events.current(),
        ["download start", "download end", "convert"]
    );
    assert_eq!(f.downloader.downloads.current().len(), 1);
    assert_eq!(f.api.requested_licenses.current(), [B1]);
    assert!(all_completed(&queue.current_snapshot()));
}

#[tokio::test]
async fn different_books_run_in_parallel_up_to_the_cap() {
    let f = Fixture::new();
    let gate = AsyncGate::default();
    let in_flight = Arc::new(Locked::new((0, 0)));
    let (counter, held) = (in_flight.clone(), gate.clone());
    f.downloader.on_download(move |_, destination, _| {
        let (counter, held) = (counter.clone(), held.clone());
        async move {
            counter.with(|(now, peak)| {
                *now += 1;
                *peak = (*peak).max(*now);
            });
            held.wait().await;
            counter.with(|(now, _)| *now -= 1);
            write(&destination, "")
        }
    });
    let (queue, _) = f.queue(3);
    queue.enqueue(&books(5), JobKind::Download, None).unwrap();
    let watched = in_flight.clone();
    wait_until(move || watched.current().0 == 3).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(in_flight.current().1, 3);
    assert_eq!(
        states(&queue.current_snapshot())
            .iter()
            .filter(|s| **s == Queued)
            .count(),
        2
    );
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(in_flight.current().1, 3);
    assert_eq!(f.downloads_contents().len(), 5);
}

#[tokio::test]
async fn jobs_can_be_added_while_others_run() {
    let f = Fixture::new();
    let gate = f.hold_downloads();
    let (queue, _) = f.queue(1);
    let all = books(2);
    queue.enqueue(&all[..1], JobKind::Download, None).unwrap();
    f.wait_for_downloads(1).await;
    assert_eq!(
        queue
            .enqueue(&all[1..], JobKind::Download, None)
            .unwrap()
            .len(),
        1
    );
    gate.open();
    queue.wait_until_finished().await;
    let snapshot = queue.current_snapshot();
    assert_eq!(snapshot.jobs.len(), 2);
    assert!(all_completed(&snapshot));
}

#[tokio::test]
async fn enqueue_skips_work_already_covered() {
    let f = Fixture::new();
    let gate = f.hold_downloads();
    let (queue, _) = f.queue(1);
    let all = books(2);
    let (book1, book2) = (&all[..1], &all[1..]);
    let destination = Some(f.destination.path());
    assert_eq!(
        queue.enqueue(book1, JobKind::Download, None).unwrap().len(),
        1
    );
    // Same kind again: skipped.
    assert!(
        queue
            .enqueue(book1, JobKind::Download, None)
            .unwrap()
            .is_empty()
    );
    // An export after a download of the same book is new work.
    assert_eq!(
        queue
            .enqueue(book1, JobKind::Export, destination)
            .unwrap()
            .len(),
        1
    );
    // A download of a book with a pending export is covered by the export.
    assert_eq!(
        queue
            .enqueue(book2, JobKind::Export, destination)
            .unwrap()
            .len(),
        1
    );
    assert!(
        queue
            .enqueue(book2, JobKind::Download, None)
            .unwrap()
            .is_empty()
    );
    gate.open();
    queue.wait_until_finished().await;
    let kinds: Vec<JobKind> = queue
        .current_snapshot()
        .jobs
        .iter()
        .map(|job| job.kind)
        .collect();
    assert_eq!(kinds, [JobKind::Download, JobKind::Export, JobKind::Export]);
}

// MARK: Export integration

#[tokio::test]
async fn exports_sequentially_with_sanitized_names_and_keeps_downloads() {
    let f = Fixture::new();
    let order = Arc::new(Locked::new(Vec::<String>::new()));
    let record = order.clone();
    f.converter.on_convert(move |request, _| {
        let record = record.clone();
        async move {
            record.with(|all| all.push(format!("convert {}", request.metadata.asin)));
            write(&request.output, "m4b")
        }
    });
    let record = order.clone();
    f.downloader.on_download(move |source, destination, _| {
        let record = record.clone();
        async move {
            let name = source
                .path_segments()
                .unwrap()
                .next_back()
                .unwrap()
                .to_string();
            record.with(|all| all.push(format!("download {name}")));
            write(&destination, "enc")
        }
    });
    let snapshot = f
        .run(
            &[
                book_in(B1, "Dune: Messiah", Marketplace::US),
                book_in(B2, "Other", Marketplace::US),
            ],
            JobKind::Export,
        )
        .await;
    assert_eq!(
        order.current(),
        [
            "download B000000001.aaxc",
            "convert B000000001",
            "download B000000002.aaxc",
            "convert B000000002"
        ]
    );
    assert_eq!(
        f.destination.contents(),
        ["Dune - Messiah [B000000001].m4b", "Other [B000000002].m4b"]
    );
    assert_eq!(
        states(&snapshot),
        [
            f.output("Dune - Messiah [B000000001].m4b"),
            f.output("Other [B000000002].m4b")
        ]
    );
    assert!(!snapshot.is_running);
    assert_eq!(snapshot.fraction_completed(), 1.0);
    // An export downloads exactly as a download job would, and keeps the file.
    assert_eq!(
        f.downloads_contents(),
        [
            "Dune - Messiah [B000000001].aaxc",
            "Other [B000000002].aaxc"
        ]
    );
    assert_eq!(
        f.manifest_contents(),
        ["B000000001.json", "B000000002.json"]
    );
}

#[tokio::test]
async fn passes_license_and_book_to_converter() {
    let f = Fixture::new();
    f.run(&books(1), JobKind::Export).await;
    let request = f.converter.requests.current().remove(0);
    let expected = license(B1);
    assert_eq!(request.decryption, expected.decryption);
    assert_eq!(request.chapters, expected.chapters);
    assert_eq!(request.metadata, BookMetadata::from(&books(1)[0]));
    assert_eq!(
        request.input,
        f.downloads.folder.join("Book 1 [B000000001].aaxc")
    );
    // Temporary output lives next to the destination so the final move is atomic.
    assert_eq!(request.output.parent().unwrap(), f.destination.path());
    assert!(
        request
            .output
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with('.')
    );
}

#[tokio::test]
async fn requests_license_from_the_books_marketplace() {
    let f = Fixture::new();
    let snapshot = f
        .run(&[book_in(B1, "Book 1", Marketplace::UK)], JobKind::Download)
        .await;
    assert!(snapshot.jobs.iter().all(|job| job.state().is_finished()));
    assert_eq!(
        f.api.requested_license_marketplaces.current(),
        [Marketplace::UK]
    );
    assert_eq!(
        f.downloads.manifest(B1).unwrap().book.marketplace,
        Marketplace::UK
    );
}

#[tokio::test]
async fn numbers_colliding_names() {
    let f = Fixture::new();
    let existing = f.destination.join("Book 1 [B000000001].m4b");
    std::fs::write(&existing, "keep me").unwrap();
    let snapshot = f
        .run(
            &[books(1).remove(0), book_in(B1, "dup", Marketplace::US)],
            JobKind::Export,
        )
        .await;
    assert_eq!(snapshot.jobs.len(), 1); // duplicate ASINs are exported once
    assert_eq!(
        f.destination.contents(),
        ["Book 1 [B000000001] (2).m4b", "Book 1 [B000000001].m4b"]
    );
    assert_eq!(std::fs::read(existing).unwrap(), b"keep me");
}

#[tokio::test]
async fn reports_phases_and_progress() {
    let f = Fixture::new();
    let download_gate = f.hold_downloads();
    let convert_gate = AsyncGate::default();
    let held = convert_gate.clone();
    f.converter.on_convert(move |request, _| {
        let held = held.clone();
        async move {
            held.wait().await;
            write(&request.output, "")
        }
    });
    let (queue, _) = f.queue(1);
    let mut updates = queue.updates();
    queue.start(&books(2), f.destination.path()).unwrap();

    // MockDownloader reports 0.5 before waiting on the gate.
    let watched = queue.clone();
    wait_until(move || states(&watched.current_snapshot())[0] == Running(Phase::Downloading, 0.5))
        .await;
    let snapshot = queue.current_snapshot();
    assert!(snapshot.is_running);
    assert_eq!(*snapshot.jobs[1].state(), Queued);
    assert!((snapshot.fraction_completed() - (0.02 + 0.58 * 0.5) / 2.0).abs() < 0.0001);

    download_gate.open();
    let watched = queue.clone();
    wait_until(move || states(&watched.current_snapshot())[0] == Running(Phase::Converting, 0.5))
        .await;
    convert_gate.open();
    queue.wait_until_finished().await;

    let snapshot = queue.current_snapshot();
    assert_eq!(snapshot.fraction_completed(), 1.0);
    // Observers receive the latest snapshot.
    updates.changed().await.unwrap();
    assert_eq!(*updates.borrow_and_update(), snapshot);
}

// MARK: Failure integration

#[tokio::test]
async fn failure_continues_with_next_book_and_keeps_completed_exports() {
    let f = Fixture::new();
    f.api.on_license(|asin, _| async move {
        if asin == B2 {
            return Err(ExporterError::UnsupportedDrm {
                asin,
                drm_type: "Widevine".into(),
            });
        }
        Ok(license(&asin))
    });
    let snapshot = f.run(&books(3), JobKind::Export).await;
    assert_eq!(
        states(&snapshot),
        [
            f.output("Book 1 [B000000001].m4b"),
            Failed("Book 2: This title uses unsupported DRM (Widevine).".into()),
            f.output("Book 3 [B000000003].m4b"),
        ]
    );
    assert_eq!(
        f.destination.contents(),
        ["Book 1 [B000000001].m4b", "Book 3 [B000000003].m4b"]
    );
}

async fn download_failure_keeps_recovery_for_the_next_download(kind: JobKind) {
    let f = Fixture::new();
    f.fail_downloads();
    let snapshot = f.run(&books(1), kind).await;
    assert_eq!(
        states(&snapshot),
        [Failed("Book 1: Download failed: HTTP 500".into())]
    );
    assert!(f.downloads_contents().is_empty());
    assert_eq!(f.manifest_contents(), ["B000000001.partial"]);
    let record = f.downloads.partial_download(B1).unwrap();
    assert_eq!(record.customer_id, session().customer_id);
    assert_eq!(
        record.recovery,
        DownloadRecovery {
            resume_data: Some(RESUME_TOKEN.to_vec()),
            progress: Some(TransferProgress::new(50, 100)),
        }
    );
    assert!(f.destination.contents().is_empty());
    assert!(f.converter.requests.current().is_empty());
}

#[tokio::test]
async fn download_job_failure_keeps_recovery() {
    download_failure_keeps_recovery_for_the_next_download(JobKind::Download).await;
}

#[tokio::test]
async fn export_job_failure_keeps_recovery() {
    download_failure_keeps_recovery_for_the_next_download(JobKind::Export).await;
}

#[tokio::test]
async fn next_download_reuses_recovery_with_a_fresh_license() {
    let f = Fixture::new();
    f.fail_downloads();
    let (queue, _) = f.queue(1);
    queue.enqueue(&books(1), JobKind::Download, None).unwrap();
    queue.wait_until_finished().await;
    f.downloader
        .on_download(|_, destination, _| async move { write(&destination, "encrypted") });
    queue.enqueue(&books(1), JobKind::Download, None).unwrap();
    queue.wait_until_finished().await;

    assert_eq!(
        states(&queue.current_snapshot()),
        [Completed(
            f.downloads.folder.join("Book 1 [B000000001].aaxc")
        )]
    );
    let resumes = f.downloader.resumes.current();
    assert_eq!(resumes.first().unwrap(), &None);
    assert_eq!(resumes.last().unwrap().as_deref(), Some(RESUME_TOKEN));
    assert_eq!(f.api.requested_licenses.current(), [B1, B1]);
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aaxc"]);
    assert_eq!(f.manifest_contents(), ["B000000001.json"]);
}

fn recovery_record(customer: &str, decryption: Decryption) -> PartialDownload {
    PartialDownload::new(
        books(1).remove(0),
        customer,
        "aaxc",
        DownloadRecovery {
            resume_data: Some(RESUME_TOKEN.to_vec()),
            progress: None,
        },
        decryption,
        reference_date(),
    )
}

#[tokio::test]
async fn another_accounts_recovery_is_discarded() {
    let f = Fixture::new();
    f.downloads.ensure_folder().unwrap();
    std::fs::write(f.downloads.partial_file_path(B1), "theirs").unwrap();
    f.downloads
        .write_partial_download(&recovery_record(
            "amzn1.account.SOMEONE.ELSE",
            license(B1).decryption,
        ))
        .unwrap();
    let snapshot = f.run(&books(1), JobKind::Download).await;
    let file = f.downloads.folder.join("Book 1 [B000000001].aaxc");
    assert_eq!(states(&snapshot), [Completed(file.clone())]);
    assert_eq!(f.downloader.resumes.current(), [None]);
    assert_eq!(std::fs::read(file).unwrap(), b"encrypted");
    assert_eq!(f.manifest_contents(), ["B000000001.json"]);
}

#[tokio::test]
async fn incompatible_license_discards_the_token() {
    let f = Fixture::new();
    f.downloads.ensure_folder().unwrap();
    let other_keys = Decryption::Aaxc {
        key: vec![99; 16],
        iv: vec![98; 16],
    };
    f.downloads
        .write_partial_download(&recovery_record(&session().customer_id, other_keys))
        .unwrap();
    f.run(&books(1), JobKind::Download).await;
    assert_eq!(f.downloader.resumes.current(), [None]);
    assert!(f.downloads.partial_downloads().is_empty());
}

async fn suspend_during_download_keeps_recovery(kind: JobKind) {
    let f = Fixture::new();
    let gate = f.interrupt_downloads();
    let (queue, _) = f.queue(1);
    queue
        .enqueue(&books(2), kind, Some(f.destination.path()))
        .unwrap();
    f.wait_for_downloads(1).await;
    queue.suspend();
    gate.open();
    queue.wait_until_finished().await;
    assert_eq!(states(&queue.current_snapshot()), [Cancelled, Cancelled]);
    assert!(f.downloads_contents().is_empty());
    assert_eq!(f.manifest_contents(), ["B000000001.partial"]);
    assert_eq!(
        f.downloads
            .partial_download(B1)
            .unwrap()
            .recovery
            .resume_data
            .as_deref(),
        Some(RESUME_TOKEN)
    );
    assert!(f.destination.contents().is_empty());
}

#[tokio::test]
async fn suspending_a_download_job_keeps_recovery() {
    suspend_during_download_keeps_recovery(JobKind::Download).await;
}

#[tokio::test]
async fn suspending_an_export_job_keeps_recovery() {
    suspend_during_download_keeps_recovery(JobKind::Export).await;
}

#[tokio::test]
async fn conversion_failure_removes_partial_output_but_keeps_download() {
    let f = Fixture::new();
    f.converter.on_convert(|request, _| async move {
        write(&request.output, "half")?;
        Err(ExporterError::Conversion("bad packet".into()))
    });
    let snapshot = f.run(&books(1), JobKind::Export).await;
    assert_eq!(
        states(&snapshot),
        [Failed("Book 1: Conversion failed: bad packet".into())]
    );
    assert!(f.destination.contents().is_empty());
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aaxc"]);
}

#[tokio::test]
async fn converter_without_output_fails() {
    let f = Fixture::new();
    f.converter.on_convert(|_, _| async { Ok(()) });
    let snapshot = f.run(&books(1), JobKind::Export).await;
    assert_eq!(
        states(&snapshot),
        [Failed(
            "Book 1: Conversion failed: The converter did not produce an output file.".into()
        )]
    );
}

#[tokio::test]
async fn denied_license_is_title_specific() {
    let f = Fixture::new();
    f.api.on_license(|asin, _| async move {
        Err(ExporterError::LicenseDenied {
            asin,
            reason: "not owned".into(),
        })
    });
    let snapshot = f.run(&books(1), JobKind::Export).await;
    assert_eq!(
        states(&snapshot),
        [Failed(
            "Book 1: The download license was denied: not owned".into()
        )]
    );
    assert!(f.downloader.downloads.current().is_empty());
}

// MARK: Progress state

fn progress_state() -> (ExportQueueState, Uuid, Uuid) {
    let job = ExportJob::new(book(B1), JobKind::Export);
    let id = job.id;
    let mut state = ExportQueueState::default();
    state.snapshot.jobs = vec![job];
    state.report(id, ExportProgressEvent::Phase(Phase::Downloading));
    let invocation = Uuid::new_v4();
    state.report(id, ExportProgressEvent::BeginDownload(invocation));
    (state, id, invocation)
}

fn progress(invocation: Uuid, attempt: i64, bytes: i64) -> ExportProgressEvent {
    ExportProgressEvent::Download(
        invocation,
        DownloadProgress {
            attempt,
            transferred: TransferProgress::new(bytes, 100),
        },
    )
}

fn received(state: &ExportQueueState) -> Option<i64> {
    state.snapshot.jobs[0]
        .transferred()
        .map(|t| t.bytes_received)
}

#[test]
fn fallback_resets_bytes_and_ignores_older_attempts() {
    let (mut state, id, invocation) = progress_state();
    state.report(id, progress(invocation, 0, 80));
    assert_eq!(received(&state), Some(80));
    state.report(id, progress(invocation, 1, 0));
    assert_eq!(
        *state.snapshot.jobs[0].state(),
        Running(Phase::Downloading, 0.0)
    );
    assert_eq!(received(&state), Some(0));
    state.report(id, progress(invocation, 1, 10));
    state.report(id, progress(invocation, 0, 95));
    state.report(id, progress(invocation, 1, 5));
    assert_eq!(
        *state.snapshot.jobs[0].state(),
        Running(Phase::Downloading, 0.1)
    );
    assert_eq!(received(&state), Some(10));
    assert_eq!(
        state.snapshot.jobs[0].first_phase(),
        Some(Phase::Downloading)
    );
}

#[test]
fn completed_error_body_does_not_stick_at_full_progress() {
    let (mut state, id, invocation) = progress_state();
    state.report(id, progress(invocation, 0, 100));
    // A callback for the new attempt can arrive before its initial zero event.
    state.report(id, progress(invocation, 1, 10));
    state.report(id, progress(invocation, 1, 0));
    assert_eq!(
        *state.snapshot.jobs[0].state(),
        Running(Phase::Downloading, 0.1)
    );
}

#[test]
fn ended_invocations_and_finished_jobs_reject_callbacks() {
    let (mut state, id, invocation) = progress_state();
    state.report(id, ExportProgressEvent::EndDownload(invocation));
    state.report(id, progress(invocation, 1, 90));
    assert_eq!(received(&state), None);
    let next = Uuid::new_v4();
    state.report(id, ExportProgressEvent::BeginDownload(next));
    state.report(id, progress(next, 0, 20));
    state.report(id, progress(invocation, 2, 100));
    assert_eq!(received(&state), Some(20));
    state.update(id, Cancelled);
    state.report(id, progress(next, 1, 99));
    assert_eq!(*state.snapshot.jobs[0].state(), Cancelled);
}

#[tokio::test]
async fn stream_starts_with_current_snapshot_and_coalesces_updates() {
    let (mut state, id, _) = progress_state();
    state.publish();
    let mut updates = state.updates();
    updates.changed().await.unwrap();
    assert_eq!(*updates.borrow_and_update(), state.snapshot);
    state.update(id, Failed("failure".into()));
    state.publish();
    state.snapshot.is_running = true;
    state.publish();
    updates.changed().await.unwrap();
    assert_eq!(*updates.borrow_and_update(), state.snapshot);
}

// MARK: Scheduling

async fn cancellation_retains_slot_through_cleanup(cancel_all: bool) {
    let f = Fixture::new();
    let started = AsyncGate::default();
    let cleanup = AsyncGate::default();
    let calls = Arc::new(Locked::new(0));
    let (open, wait, counter) = (started.clone(), cleanup.clone(), calls.clone());
    f.downloader.on_download(move |_, destination, cancel| {
        let (open, wait) = (open.clone(), wait.clone());
        let first = counter.with(|count| {
            *count += 1;
            *count == 1
        });
        async move {
            if first {
                open.open();
                wait.wait().await;
                check(&cancel)?;
            }
            write(&destination, "audio")
        }
    });
    let all = books(3);
    let (queue, _) = f.queue(1);
    let first = queue.enqueue(&all[..1], JobKind::Download, None).unwrap()[0];
    started.wait().await;
    queue.start(&all[..2], f.destination.path()).unwrap();
    if cancel_all {
        queue.cancel_all();
    } else {
        queue.cancel(first);
    }
    queue.enqueue(&all[2..], JobKind::Download, None).unwrap();
    let waiting = queue.current_snapshot();
    assert!(waiting.is_running);
    assert_eq!(*waiting.jobs.last().unwrap().state(), Queued);
    assert_eq!(f.api.requested_licenses.current(), [all[0].asin.clone()]);
    assert_eq!(calls.current(), 1);
    cleanup.open();
    queue.wait_until_finished().await;
    let finished = queue.current_snapshot();
    assert!(!finished.is_running);
    assert_eq!(*finished.jobs[0].state(), Cancelled);
    let cancelled = states(&finished)
        .iter()
        .filter(|s| **s == Cancelled)
        .count();
    assert_eq!(cancelled, if cancel_all { 3 } else { 1 });
    assert!(finished.jobs.last().unwrap().state().is_finished());
    assert!(f.downloads.scan_downloads().contains_key(&all[2].asin));
}

#[tokio::test]
async fn cancelling_all_retains_slot_through_cleanup() {
    cancellation_retains_slot_through_cleanup(true).await;
}

#[tokio::test]
async fn cancelling_one_retains_slot_through_cleanup() {
    cancellation_retains_slot_through_cleanup(false).await;
}

#[tokio::test]
async fn cancelling_last_authentication_waiter_admits_offline_export() {
    let f = Fixture::new();
    let all = books(2);
    f.keep(&all[1], None, true);
    f.reject_everything();
    let (queue, _) = f.queue(1);
    queue.start(&all, f.destination.path()).unwrap();
    wait_for_pause(&queue).await;
    let id = queue.current_snapshot().jobs[0].id;
    queue.cancel(id);
    queue.cancel(id);
    queue.resume_after_authentication();
    queue.wait_until_finished().await;
    let finished = queue.current_snapshot();
    assert_eq!(
        states(&finished),
        [Cancelled, f.output("Book 2 [B000000002].m4b")]
    );
    assert!(!finished.is_waiting_for_authentication);
}

// MARK: Validation

#[tokio::test]
async fn rejects_invalid_enqueues() {
    let f = Fixture::new();
    let (queue, _) = f.queue(1);
    assert_eq!(
        queue.start(&[], f.destination.path()),
        Err(StartError::NothingToExport)
    );
    let missing = f.destination.join("missing");
    assert_eq!(
        queue.start(&books(1), &missing),
        Err(StartError::DestinationUnavailable(
            missing.display().to_string()
        ))
    );
    queue.start(&books(1), f.destination.path()).unwrap();
    queue.wait_until_finished().await;
    queue.start(&books(1), f.destination.path()).unwrap(); // a finished queue can start again
    queue.wait_until_finished().await;
}

#[tokio::test]
async fn creates_missing_download_folder() {
    let f = Fixture::new();
    assert!(!f.downloads.folder.exists());
    f.run(&books(1), JobKind::Download).await;
    assert_eq!(f.downloads_contents(), ["Book 1 [B000000001].aaxc"]);
}

#[tokio::test]
async fn uncreatable_download_folder_is_reported() {
    let f = Fixture::new();
    // A regular file where the folder should be.
    let blocker = f.work.join("NotAFolder");
    std::fs::write(&blocker, "").unwrap();
    let downloads = LocalDownloads::new(blocker.clone(), f.work.join("M"));
    let queue = f.queue_with(f.auth(), downloads, 3);
    assert_eq!(
        queue.enqueue(&books(1), JobKind::Download, None),
        Err(StartError::DestinationUnavailable(
            blocker.display().to_string()
        ))
    );
    assert!(queue.current_snapshot().jobs.is_empty());
}
