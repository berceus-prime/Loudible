//! Schedules downloads and exports.
//!
//! - A **download** fetches the encrypted file (license, download, manifest, move into place)
//!   and keeps it in `LocalDownloads::folder`.
//! - An **export** converts the kept file to M4B. A book that is not downloaded yet is downloaded
//!   first, exactly as a download job would, and the file is kept.
//!
//! One job runs at a time in queue order. Work can be added while it runs. A failed job does not
//! stop the next one. A job needing sign-in holds the queue until resumed or cancelled.

pub mod book_export_workflow;
pub mod download_workflow;
pub mod execution_context;
pub mod file_finalizer;
pub mod license_provider;
pub mod state;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use self::book_export_workflow::BookExportWorkflow;
use self::download_workflow::DownloadWorkflow;
use self::execution_context::ExportExecutionContext;
use self::file_finalizer::ExportFileFinalizer;
use self::license_provider::LicenseProvider;
use self::state::ExportQueueState;
use crate::local_downloads::is_writable_directory;
use crate::{
    AudibleApi, AudioConverter, Audiobook, AuthenticationCoordinator, ExportJob, ExporterError,
    FileDownloader, JobKind, JobState, LocalDownloads, Phase,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    NothingToExport,
    DestinationUnavailable(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub jobs: Vec<ExportJob>,
    pub is_running: bool,
    pub is_waiting_for_authentication: bool,
}

impl Snapshot {
    /// Completion across all jobs (0...1).
    pub fn fraction_completed(&self) -> f64 {
        if self.jobs.is_empty() {
            return 0.0;
        }
        let total: f64 = self.jobs.iter().map(ExportJob::fraction_completed).sum();
        total / self.jobs.len() as f64
    }
}

struct Core {
    pending: VecDeque<Uuid>,
    state: ExportQueueState,
    running: Option<(Uuid, CancellationToken)>,
    export_destinations: HashMap<Uuid, PathBuf>,
    authentication_waiter: Option<oneshot::Sender<()>>,
    finish_waiters: Vec<oneshot::Sender<()>>,
}

struct Inner {
    downloads: LocalDownloads,
    download: DownloadWorkflow,
    exporter: BookExportWorkflow,
    core: Mutex<Core>,
}

/// Cheap to clone. Must be used inside a Tokio runtime. The state is only locked between awaits,
/// so calls interleave as they would on an actor.
#[derive(Clone)]
pub struct ExportQueue(Arc<Inner>);

impl ExportQueue {
    pub fn new(
        auth: AuthenticationCoordinator,
        api: Arc<dyn AudibleApi>,
        downloader: Arc<dyn FileDownloader>,
        converter: Arc<dyn AudioConverter>,
        downloads: LocalDownloads,
    ) -> Self {
        let finalizer = ExportFileFinalizer;
        let download = DownloadWorkflow {
            licenses: LicenseProvider { auth, api },
            downloader,
            downloads: downloads.clone(),
            finalizer,
        };
        let exporter = BookExportWorkflow {
            download: download.clone(),
            converter,
            finalizer,
        };
        Self(Arc::new(Inner {
            downloads,
            download,
            exporter,
            core: Mutex::new(Core {
                pending: VecDeque::new(),
                state: ExportQueueState::default(),
                running: None,
                export_destinations: HashMap::new(),
                authentication_waiter: None,
                finish_waiters: Vec::new(),
            }),
        }))
    }

    pub fn current_snapshot(&self) -> Snapshot {
        self.0.with_core(|core| core.state.snapshot.clone())
    }

    /// Yields the current snapshot first, then the latest whenever it changes.
    pub fn updates(&self) -> watch::Receiver<Snapshot> {
        self.0.with_core(|core| core.state.updates())
    }

    // MARK: Control

    /// Exports `books` into `destination`. Returns once the jobs are queued.
    pub fn start(&self, books: &[Audiobook], destination: &Path) -> Result<(), StartError> {
        self.enqueue(books, JobKind::Export, Some(destination))
            .map(|_| ())
    }

    /// Queues one job of `kind` per book and returns the ids of the jobs it added. Books that
    /// already have an unfinished job of that kind are skipped, and so is a download of a book
    /// with an unfinished export, which downloads it anyway.
    /// `export_destination` is required for `Export`, ignored for `Download`.
    pub fn enqueue(
        &self,
        books: &[Audiobook],
        kind: JobKind,
        export_destination: Option<&Path>,
    ) -> Result<Vec<Uuid>, StartError> {
        let mut seen = HashSet::new();
        let unique: Vec<&Audiobook> = books.iter().filter(|b| seen.insert(&b.asin)).collect();
        if unique.is_empty() {
            return Err(StartError::NothingToExport);
        }

        // First, so an export destination that is the downloads folder exists before it is checked.
        self.0.downloads.ensure_folder().map_err(|_| {
            StartError::DestinationUnavailable(self.0.downloads.folder.display().to_string())
        })?;
        if kind == JobKind::Export {
            let destination =
                export_destination.ok_or(StartError::DestinationUnavailable(String::new()))?;
            if !destination.is_dir() || !is_writable_directory(destination) {
                return Err(StartError::DestinationUnavailable(
                    destination.display().to_string(),
                ));
            }
        }

        let inner = &self.0;
        Ok(inner.with_core(|core| {
            // A new run starts with a clean list; finished jobs of the last run are dropped.
            if !core.state.snapshot.is_running {
                core.state.snapshot.jobs.clear();
                core.export_destinations.clear();
            }
            let pending: Vec<(String, JobKind)> = core
                .state
                .snapshot
                .jobs
                .iter()
                .filter(|job| !job.state().is_finished())
                .map(|job| (job.book.asin.clone(), job.kind))
                .collect();
            let mut added = Vec::new();
            for book in unique {
                let covered = pending.iter().any(|(asin, job_kind)| {
                    *asin == book.asin
                        && (*job_kind == kind
                            || (kind == JobKind::Download && *job_kind == JobKind::Export))
                });
                if covered {
                    continue;
                }
                let job = ExportJob::new(book.clone(), kind);
                let id = job.id;
                core.state.snapshot.jobs.push(job);
                core.pending.push_back(id);
                if let (JobKind::Export, Some(destination)) = (kind, export_destination) {
                    core.export_destinations
                        .insert(id, destination.to_path_buf());
                }
                added.push(id);
            }
            if !added.is_empty() {
                core.state.snapshot.is_running = true;
            }
            schedule(inner, core);
            added
        }))
    }

    /// Cancels every running and queued job. Completed ones are kept.
    pub fn cancel_all(&self) {
        self.0.with_core(cancel_everything);
    }

    /// Stops every job on quit or sign-out.
    pub fn suspend(&self) {
        self.cancel_all();
    }

    /// Cancels one job: a running one stops, a queued one is skipped. Finished jobs and unknown
    /// ids are ignored.
    pub fn cancel(&self, id: Uuid) {
        let inner = &self.0;
        inner.with_core(|core| {
            if let Some((_, token)) = core.running.as_ref().filter(|(running, _)| *running == id) {
                token.cancel();
            } else if let Some(index) = core.pending.iter().position(|pending| *pending == id) {
                core.pending.remove(index);
                core.state.update(id, JobState::Cancelled);
                schedule(inner, core);
            }
        });
    }

    /// Forgets the jobs of a finished run. Ignored while jobs are running.
    pub fn clear(&self) {
        self.0.with_core(|core| {
            if !core.state.snapshot.is_running {
                core.state.snapshot = Snapshot::default();
                core.export_destinations.clear();
            }
        });
    }

    /// Continues every job paused for sign-in.
    pub fn resume_after_authentication(&self) {
        let inner = &self.0;
        inner.with_core(|core| {
            if let Some(waiter) = core.authentication_waiter.take() {
                let _ = waiter.send(());
            }
            core.state.snapshot.is_waiting_for_authentication = false;
            schedule(inner, core);
        });
    }

    /// Waits until no job is queued or running.
    pub async fn wait_until_finished(&self) {
        let waiter = self.0.with_core(|core| {
            core.state.snapshot.is_running.then(|| {
                let (sender, receiver) = oneshot::channel();
                core.finish_waiters.push(sender);
                receiver
            })
        });
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
    }
}

fn cancel_everything(core: &mut Core) {
    for id in std::mem::take(&mut core.pending) {
        core.state.update(id, JobState::Cancelled);
    }
    if let Some((_, token)) = &core.running {
        token.cancel();
    }
    finish_if_idle(core);
}

fn schedule(inner: &Arc<Inner>, core: &mut Core) {
    if core.running.is_none() && !core.state.snapshot.is_waiting_for_authentication {
        if let Some(id) = core.pending.pop_front() {
            let token = CancellationToken::new();
            core.running = Some((id, token.clone()));
            let inner = inner.clone();
            tokio::spawn(async move {
                inner.process(id, token).await;
                inner.job_ended(id);
            });
        }
    }
    finish_if_idle(core);
}

fn finish_if_idle(core: &mut Core) {
    if !core.state.snapshot.is_running || core.running.is_some() || !core.pending.is_empty() {
        return;
    }
    core.state.snapshot.is_running = false;
    core.state.snapshot.is_waiting_for_authentication = false;
    for waiter in core.finish_waiters.drain(..) {
        let _ = waiter.send(());
    }
}

impl Inner {
    fn with_core<R>(&self, change: impl FnOnce(&mut Core) -> R) -> R {
        let mut core = self.core.lock().expect("no panic while holding the lock");
        let result = change(&mut core);
        core.state.publish();
        result
    }

    fn update(&self, id: Uuid, state: JobState) {
        self.with_core(|core| core.state.update(id, state));
    }

    fn job_ended(self: &Arc<Self>, id: Uuid) {
        self.with_core(|core| {
            core.running = None;
            core.export_destinations.remove(&id);
            schedule(self, core);
        });
    }

    async fn process(self: &Arc<Self>, id: Uuid, cancel: CancellationToken) {
        let job = self.with_core(|core| {
            core.state
                .snapshot
                .jobs
                .iter()
                .find(|job| job.id == id)
                .cloned()
        });
        let Some(job) = job else {
            return;
        };
        loop {
            match self.run_once(id, &job, &cancel).await {
                Ok(output) => {
                    self.update(id, JobState::Completed(output));
                    return;
                }
                Err(ExporterError::AuthenticationRequired) => {
                    self.update(id, JobState::WaitingForAuthentication);
                    self.wait_for_authentication(&cancel).await;
                    if cancel.is_cancelled() {
                        self.update(id, JobState::Cancelled);
                        return;
                    }
                    // A paused job restarts from licensing, whatever phase it then skips to.
                    self.update(id, JobState::Running(Phase::Licensing, 0.0));
                }
                Err(ExporterError::Cancelled) => {
                    self.update(id, JobState::Cancelled);
                    return;
                }
                Err(error) => {
                    let state = if cancel.is_cancelled() {
                        JobState::Cancelled
                    } else {
                        JobState::Failed(format!("{}: {error}", job.book.title))
                    };
                    self.update(id, state);
                    return;
                }
            }
        }
    }

    async fn run_once(
        self: &Arc<Self>,
        id: Uuid,
        job: &ExportJob,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, ExporterError> {
        if cancel.is_cancelled() {
            return Err(ExporterError::Cancelled);
        }
        let reporter = self.clone();
        let context = ExportExecutionContext {
            report: Arc::new(move |event| {
                reporter.with_core(|core| core.state.report(id, event));
            }),
            cancel: cancel.clone(),
        };
        match job.kind {
            JobKind::Download => Ok(self
                .download
                .run(&job.book, JobKind::Download, &context)
                .await?
                .input),
            JobKind::Export => {
                let destination = self
                    .with_core(|core| core.export_destinations.get(&id).cloned())
                    .ok_or_else(|| ExporterError::Unexpected("No export folder.".to_string()))?;
                self.exporter.run(&job.book, &destination, &context).await
            }
        }
    }

    // MARK: Authentication pause

    async fn wait_for_authentication(self: &Arc<Self>, cancel: &CancellationToken) {
        let waiter = self.with_core(|core| {
            if cancel.is_cancelled() {
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            core.authentication_waiter = Some(sender);
            core.state.snapshot.is_waiting_for_authentication = true;
            Some(receiver)
        });
        let Some(waiter) = waiter else {
            return;
        };
        tokio::select! {
            _ = waiter => {}
            _ = cancel.cancelled() => self.wake_authentication_waiter(),
        }
    }

    fn wake_authentication_waiter(self: &Arc<Self>) {
        self.with_core(|core| {
            core.authentication_waiter = None;
            core.state.snapshot.is_waiting_for_authentication = false;
            schedule(self, core);
        });
    }
}
