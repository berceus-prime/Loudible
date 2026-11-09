use uuid::Uuid;

use crate::{ExportJob, JobKind, JobState, TransferProgress};

/// The one status shown on a book's cover. When several apply, the most relevant wins:
/// running > queued > failed > exported > downloaded > none.
#[derive(Clone, Debug, PartialEq)]
pub enum BookStatus {
    None,
    /// Queued, or paused for sign-in.
    Queued(JobKind),
    /// `progress` is the job's overall completion (0...1); `transferred` is set while the audio
    /// file is downloading.
    Running {
        kind: JobKind,
        progress: f64,
        transferred: Option<TransferProgress>,
    },
    /// The latest attempt failed. Kept in memory only; cleared when the next attempt starts.
    Failed {
        kind: JobKind,
        message: String,
    },
    Downloaded,
    Exported,
}

impl BookStatus {
    /// - `jobs`: the queue's jobs; only unfinished jobs of `asin` are considered.
    /// - `failure`: the failure of the book's latest attempt, if it failed.
    pub fn resolve(
        asin: &str,
        jobs: &[ExportJob],
        failure: Option<(JobKind, &str)>,
        is_downloaded: bool,
        is_exported: bool,
    ) -> Self {
        let active: Vec<&ExportJob> = jobs
            .iter()
            .filter(|job| job.book.asin == asin && !job.state().is_finished())
            .collect();
        if let Some(job) = active
            .iter()
            .find(|job| matches!(job.state(), JobState::Running(..)))
        {
            return Self::Running {
                kind: job.kind,
                progress: job.fraction_completed(),
                transferred: job.transferred(),
            };
        }
        if let Some(job) = active.first() {
            return Self::Queued(job.kind);
        }
        if let Some((kind, message)) = failure {
            return Self::Failed {
                kind,
                message: message.to_string(),
            };
        }
        if is_exported {
            return Self::Exported;
        }
        if is_downloaded {
            return Self::Downloaded;
        }
        Self::None
    }
}

/// Something the user can do to one book from its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BookAction {
    Download,
    Cancel { job_id: Uuid, kind: JobKind },
    Export,
    ShowDownload,
    ShowExport,
    DeleteDownload,
}

/// A menu entry: shown items only, with a reason when it is disabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookMenuItem {
    pub action: BookAction,
    pub is_enabled: bool,
    pub disabled_reason: Option<String>,
}

impl BookMenuItem {
    pub fn new(action: BookAction) -> Self {
        Self {
            action,
            is_enabled: true,
            disabled_reason: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookMenuContext {
    pub is_downloaded: bool,
    pub is_exported: bool,
    pub is_in_library: bool,
    pub has_manifest: bool,
    pub is_signed_in: bool,
}

/// Which menu items a book offers. Items that don't apply are left out, except Export of a
/// downloaded book, which is disabled with a reason. Export of a book that is not downloaded yet
/// downloads it first, so it is offered alongside Download.
pub fn book_menu_items(
    asin: &str,
    jobs: &[ExportJob],
    context: &BookMenuContext,
) -> Vec<BookMenuItem> {
    let active: Vec<&ExportJob> = jobs
        .iter()
        .filter(|job| job.book.asin == asin && !job.state().is_finished())
        .collect();
    let mut items = Vec::new();
    if !context.is_downloaded && context.is_in_library && active.is_empty() {
        items.push(BookMenuItem::new(BookAction::Download));
    }
    for job in &active {
        items.push(BookMenuItem::new(BookAction::Cancel {
            job_id: job.id,
            kind: job.kind,
        }));
    }
    if context.is_downloaded {
        let reason = export_blocker(!active.is_empty(), context);
        items.push(BookMenuItem {
            action: BookAction::Export,
            is_enabled: reason.is_none(),
            disabled_reason: reason.map(str::to_string),
        });
        items.push(BookMenuItem::new(BookAction::ShowDownload));
    } else if context.is_in_library && active.is_empty() {
        items.push(BookMenuItem::new(BookAction::Export));
    }
    if context.is_exported {
        items.push(BookMenuItem::new(BookAction::ShowExport));
    }
    if context.is_downloaded && active.is_empty() {
        items.push(BookMenuItem::new(BookAction::DeleteDownload));
    }
    items
}

fn export_blocker(has_active_job: bool, context: &BookMenuContext) -> Option<&'static str> {
    // Exporting books that have left the library is out of scope.
    if !context.is_in_library {
        return Some("This book is no longer in your library.");
    }
    if has_active_job {
        return Some("This book is already being downloaded or exported.");
    }
    if !context.has_manifest && !context.is_signed_in {
        return Some("Sign in to Audible to get the keys for this download.");
    }
    None
}
