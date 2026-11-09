use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use loudible_core::export_queue::Snapshot;
use loudible_core::{Audiobook, ExportJob, ExportQueue, JobKind, JobState, StartError};
use uuid::Uuid;

use crate::session_view_model::ErrorSink;
use crate::{AppError, Changes};

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    error_message: Option<String>,
    failures: HashMap<String, (JobKind, String)>,
    notified_failures: HashSet<Uuid>,
    is_observing: bool,
    on_error: Option<ErrorSink>,
    on_job_finished: Option<Rc<dyn Fn()>>,
}

struct Inner {
    queue: ExportQueue,
    export_folder: PathBuf,
    state: RefCell<State>,
    changes: Changes,
}

/// Download and export jobs, their progress, and cancellation.
#[derive(Clone)]
pub struct ExportViewModel(Rc<Inner>);

/// Job failures read "Title: reason"; next to the book the title is redundant.
fn reason(message: &str, title: &str) -> String {
    message
        .strip_prefix(&format!("{title}: "))
        .unwrap_or(message)
        .to_string()
}

impl ExportViewModel {
    /// `export_folder` is where exported M4B files go: the downloads folder, next to the
    /// encrypted files.
    pub fn new(queue: ExportQueue, export_folder: PathBuf, changes: Changes) -> Self {
        Self(Rc::new(Inner {
            queue,
            export_folder,
            state: RefCell::default(),
            changes,
        }))
    }

    fn read<R>(&self, body: impl FnOnce(&State) -> R) -> R {
        body(&self.0.state.borrow())
    }

    pub fn set_on_error(&self, sink: impl Fn(AppError) + 'static) {
        self.0.state.borrow_mut().on_error = Some(Rc::new(sink));
    }

    /// Called whenever another job finishes, e.g. to rescan the folders.
    pub fn set_on_job_finished(&self, callback: impl Fn() + 'static) {
        self.0.state.borrow_mut().on_job_finished = Some(Rc::new(callback));
    }

    fn report(&self, error: AppError) {
        let sink = self.read(|state| state.on_error.clone());
        if let Some(sink) = sink {
            sink(error);
        }
    }

    /// Starts mirroring the queue's progress.
    pub fn start(&self) {
        if std::mem::replace(&mut self.0.state.borrow_mut().is_observing, true) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        let mut updates = self.0.queue.updates();
        tokio::task::spawn_local(async move {
            loop {
                let snapshot = updates.borrow_and_update().clone();
                let Some(inner) = weak.upgrade() else { break };
                Self(inner).apply(snapshot);
                if updates.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    fn apply(&self, new: Snapshot) {
        let mut errors = Vec::new();
        let (finished_more, waiting_changed) = {
            let mut state = self.0.state.borrow_mut();
            let finished = |snapshot: &Snapshot| {
                snapshot
                    .jobs
                    .iter()
                    .filter(|job| job.state().is_finished())
                    .count()
            };
            for job in &new.jobs {
                if let JobState::Failed(message) = job.state()
                    && state.notified_failures.insert(job.id)
                {
                    let book_title = job.book.title.clone();
                    let technical_error = message.clone();
                    errors.push(match job.kind {
                        JobKind::Download => AppError::Download {
                            book_title,
                            technical_error,
                        },
                        JobKind::Export => AppError::Export {
                            book_title,
                            technical_error,
                        },
                    });
                }
            }
            // The last job of each book in the list decides whether it is failed.
            let mut latest: HashMap<&str, &ExportJob> = HashMap::new();
            for job in &new.jobs {
                latest.insert(&job.book.asin, job);
            }
            for (asin, job) in latest {
                if let JobState::Failed(message) = job.state() {
                    let text = reason(message, &job.book.title);
                    state.failures.insert(asin.to_string(), (job.kind, text));
                } else {
                    state.failures.remove(asin);
                }
            }
            let finished_more = finished(&new) > finished(&state.snapshot);
            let waiting_changed =
                new.is_waiting_for_authentication != state.snapshot.is_waiting_for_authentication;
            state.snapshot = new;
            (finished_more, waiting_changed)
        };
        errors.into_iter().for_each(|error| self.report(error));
        self.0.changes.mark();
        if waiting_changed {
            self.0.changes.mark_session();
        }
        if finished_more {
            let callback = self.read(|state| state.on_job_finished.clone());
            if let Some(callback) = callback {
                callback();
            }
        }
    }

    fn apply_current(&self) {
        self.apply(self.0.queue.current_snapshot());
    }

    // MARK: Derived state

    pub fn export_folder(&self) -> &Path {
        &self.0.export_folder
    }

    pub fn jobs(&self) -> Vec<ExportJob> {
        self.read(|state| state.snapshot.jobs.clone())
    }

    pub fn is_running(&self) -> bool {
        self.read(|state| state.snapshot.is_running)
    }

    pub fn is_waiting_for_authentication(&self) -> bool {
        self.read(|state| state.snapshot.is_waiting_for_authentication)
    }

    /// The failure of each book's latest attempt, by ASIN. Survives the queue dropping finished
    /// jobs, and is cleared when the book's next attempt starts.
    pub fn failures(&self) -> HashMap<String, (JobKind, String)> {
        self.read(|state| state.failures.clone())
    }

    pub fn failure(&self, asin: &str) -> Option<(JobKind, String)> {
        self.read(|state| state.failures.get(asin).cloned())
    }

    pub fn error_message(&self) -> Option<String> {
        self.read(|state| state.error_message.clone())
    }

    pub fn set_error_message(&self, message: Option<String>) {
        self.0.state.borrow_mut().error_message = message;
        self.0.changes.mark();
    }

    /// The ASINs with an unfinished job.
    pub(crate) fn busy_asins(&self) -> HashSet<String> {
        self.read(|state| {
            state
                .snapshot
                .jobs
                .iter()
                .filter(|job| !job.state().is_finished())
                .map(|job| job.book.asin.clone())
                .collect()
        })
    }

    pub fn active_job_count(&self) -> usize {
        self.read(|state| {
            state
                .snapshot
                .jobs
                .iter()
                .filter(|job| !job.state().is_finished())
                .count()
        })
    }

    /// Shown above the library while jobs wait for the user to sign in again.
    pub fn notice(&self) -> Option<&'static str> {
        self.is_waiting_for_authentication()
            .then_some("Paused: sign in to Audible to continue.")
    }

    /// Whether an export or a job that hasn't started is unfinished: the work `suspend()` loses.
    /// Running downloads keep their bytes.
    pub fn has_work_suspend_loses(&self) -> bool {
        self.read(|state| {
            state.snapshot.jobs.iter().any(|job| match job.state() {
                JobState::Running(..) => job.kind == JobKind::Export,
                JobState::Queued | JobState::WaitingForAuthentication => true,
                JobState::Completed(_) | JobState::Failed(_) | JobState::Cancelled => false,
            })
        })
    }

    // MARK: Actions

    /// Exports books to M4B, downloading the ones not downloaded yet.
    pub fn export(&self, books: &[Audiobook]) {
        self.enqueue(books, JobKind::Export);
    }

    /// Downloads books and keeps the encrypted files.
    pub fn download(&self, books: &[Audiobook]) {
        self.enqueue(books, JobKind::Download);
    }

    fn enqueue(&self, books: &[Audiobook], kind: JobKind) {
        self.set_error_message(None);
        match self
            .0
            .queue
            .enqueue(books, kind, Some(&self.0.export_folder))
        {
            Ok(_) => self.apply_current(),
            Err(StartError::DestinationUnavailable(path)) => self.report(AppError::Enqueue {
                kind,
                book_count: books.len(),
                technical_error: format!(
                    "The download folder could not be created or is not writable: {path}"
                ),
            }),
            Err(StartError::NothingToExport) => {
                self.set_error_message(Some("Select at least one book to export.".to_string()));
            }
        }
    }

    /// Cancels every job and removes them all from the list, completed ones included.
    pub async fn cancel_all(&self) {
        self.0.queue.cancel_all();
        self.clear_when_finished().await;
    }

    /// Stops every job and removes them all from the list, like `cancel_all()`, but downloads in
    /// progress keep their bytes, to be continued later.
    pub async fn suspend(&self) {
        self.0.queue.suspend();
        self.clear_when_finished().await;
    }

    async fn clear_when_finished(&self) {
        self.0.queue.wait_until_finished().await;
        self.0.queue.clear();
        self.apply_current();
    }

    /// Cancels one job; the others continue.
    pub fn cancel(&self, job_id: Uuid) {
        self.0.queue.cancel(job_id);
    }

    pub fn resume_after_authentication(&self) {
        self.0.queue.resume_after_authentication();
    }
}
