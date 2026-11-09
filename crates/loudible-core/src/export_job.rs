use std::path::PathBuf;

use uuid::Uuid;

use crate::{Audiobook, TransferProgress};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Fetch and keep the encrypted file.
    Download,
    /// Convert to M4B, downloading first if the book is not downloaded yet.
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Licensing,
    Downloading,
    Converting,
    Finalizing,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    /// The progress is the fraction (0...1) of the current phase.
    Running(Phase, f64),
    /// Paused until the user signs in again; the job restarts from licensing.
    WaitingForAuthentication,
    Completed(PathBuf),
    Failed(String),
    Cancelled,
}

impl JobState {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Completed(_) | Self::Failed(_) | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InvalidTransition {
    pub from: JobState,
    pub to: JobState,
}

/// Progress of one download or export of one audiobook. State changes go through `transition`,
/// which rejects moves the queue must never make (for example, leaving a finished state).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportJob {
    /// Unique per job: one book can have a download and an export job at the same time.
    pub id: Uuid,
    pub kind: JobKind,
    pub book: Audiobook,
    state: JobState,
    first_phase: Option<Phase>,
    transferred: Option<TransferProgress>,
}

impl ExportJob {
    pub fn new(book: Audiobook, kind: JobKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            book,
            state: JobState::Queued,
            first_phase: None,
            transferred: None,
        }
    }

    pub fn state(&self) -> &JobState {
        &self.state
    }

    /// The phase the job started in. An export of a downloaded book starts at `Converting`, and
    /// its progress is measured from there.
    pub fn first_phase(&self) -> Option<Phase> {
        self.first_phase
    }

    /// Bytes of the audio download so far; only while downloading and once its size is known.
    pub fn transferred(&self) -> Option<TransferProgress> {
        self.transferred
    }

    pub fn transition(&mut self, state: JobState) -> Result<(), InvalidTransition> {
        self.transition_with(state, None)
    }

    /// `transferred` is the byte count behind a `Downloading` state; ignored otherwise.
    pub fn transition_with(
        &mut self,
        state: JobState,
        transferred: Option<TransferProgress>,
    ) -> Result<(), InvalidTransition> {
        if !Self::allows(&self.state, &state) {
            return Err(InvalidTransition {
                from: self.state.clone(),
                to: state,
            });
        }
        if let (None, JobState::Running(phase, _)) = (self.first_phase, &state) {
            self.first_phase = Some(*phase);
        }
        self.transferred = match state {
            JobState::Running(Phase::Downloading, _) => transferred,
            _ => None,
        };
        self.state = state;
        Ok(())
    }

    /// Only an identified new transfer attempt may reset progress within downloading.
    pub(crate) fn restart_download(&mut self) {
        if matches!(self.state, JobState::Running(Phase::Downloading, _)) {
            self.state = JobState::Running(Phase::Downloading, 0.0);
            self.transferred = None;
        }
    }

    fn allows(old: &JobState, new: &JobState) -> bool {
        use JobState::*;
        match (old, new) {
            (Completed(_) | Failed(_) | Cancelled, _) => false,
            // Work already done (a kept download, stored keys) lets a job start in any phase.
            (Queued, Running(..) | WaitingForAuthentication | Failed(_) | Cancelled) => true,
            (Running(old_phase, old_progress), Running(new_phase, new_progress)) => {
                // Phases only move forward; progress within a phase never goes backwards.
                new_phase > old_phase || (new_phase == old_phase && new_progress >= old_progress)
            }
            (Running(..), WaitingForAuthentication | Completed(_) | Failed(_) | Cancelled) => true,
            (WaitingForAuthentication, Running(Phase::Licensing, _) | Failed(_) | Cancelled) => {
                true
            }
            _ => false,
        }
    }

    /// Overall completion of this job (0...1), weighting download and conversion most and
    /// counting only the phases from `first_phase` on.
    pub fn fraction_completed(&self) -> f64 {
        match self.state {
            JobState::Queued
            | JobState::WaitingForAuthentication
            | JobState::Failed(_)
            | JobState::Cancelled => 0.0,
            JobState::Completed(_) => 1.0,
            JobState::Running(phase, progress) => {
                let (start, weight) = self.phase_share(phase);
                let raw = start + weight * progress.clamp(0.0, 1.0);
                let (origin, _) = self.phase_share(self.first_phase.unwrap_or(Phase::Licensing));
                if origin < 1.0 {
                    ((raw - origin) / (1.0 - origin)).max(0.0)
                } else {
                    raw
                }
            }
        }
    }

    /// Where `phase` starts in the job's overall progress, and how much of it the phase is.
    /// A download never converts, so `Converting` is given no weight there.
    fn phase_share(&self, phase: Phase) -> (f64, f64) {
        match (self.kind, phase) {
            (_, Phase::Licensing) => (0.0, 0.02),
            (JobKind::Export, Phase::Downloading) => (0.02, 0.58),
            (JobKind::Export, Phase::Converting) => (0.60, 0.38),
            (JobKind::Export, Phase::Finalizing) => (0.98, 0.02),
            (JobKind::Download, Phase::Downloading) => (0.02, 0.96),
            (JobKind::Download, Phase::Converting) => (0.98, 0.0),
            (JobKind::Download, Phase::Finalizing) => (0.98, 0.02),
        }
    }
}
