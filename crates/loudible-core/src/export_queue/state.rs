use std::collections::HashMap;

use tokio::sync::watch;
use uuid::Uuid;

use super::Snapshot;
use super::execution_context::ExportProgressEvent;
use crate::{JobState, Phase, TransferProgress};

/// Queue-owned application state; workflows never mutate it directly.
pub struct ExportQueueState {
    pub snapshot: Snapshot,
    published: watch::Sender<Snapshot>,
    transfers: HashMap<Uuid, Uuid>,
}

impl Default for ExportQueueState {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            published: watch::Sender::new(Snapshot::default()),
            transfers: HashMap::new(),
        }
    }
}

impl ExportQueueState {
    /// Yields the current snapshot first, then the latest whenever it changes.
    pub fn updates(&self) -> watch::Receiver<Snapshot> {
        let mut receiver = self.published.subscribe();
        receiver.mark_changed();
        receiver
    }

    /// Tells observers about the snapshot, if it changed.
    pub fn publish(&self) {
        self.published.send_if_modified(|published| {
            let changed = *published != self.snapshot;
            if changed {
                published.clone_from(&self.snapshot);
            }
            changed
        });
    }

    fn index_of(&self, id: Uuid) -> Option<usize> {
        self.snapshot.jobs.iter().position(|job| job.id == id)
    }

    pub fn update(&mut self, id: Uuid, state: JobState) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let is_finished = state.is_finished();
        let _ = self.snapshot.jobs[index].transition(state);
        if is_finished {
            self.transfers.remove(&id);
        }
    }

    pub fn report(&mut self, id: Uuid, event: ExportProgressEvent) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        if self.snapshot.jobs[index].state().is_finished() {
            return;
        }
        let is_downloading = matches!(
            self.snapshot.jobs[index].state(),
            JobState::Running(Phase::Downloading, _)
        );
        match event {
            ExportProgressEvent::Phase(phase) => {
                self.transfers.remove(&id);
                self.update(id, JobState::Running(phase, 0.0));
            }
            ExportProgressEvent::BeginDownload(invocation) => {
                if is_downloading {
                    self.transfers.insert(id, invocation);
                }
            }
            ExportProgressEvent::EndDownload(invocation) => {
                if self.transfers.get(&id) == Some(&invocation) {
                    self.transfers.remove(&id);
                }
            }
            ExportProgressEvent::Download(invocation, progress) => {
                let Some(&current) = self.transfers.get(&id) else {
                    return;
                };
                if current != invocation || !is_downloading {
                    return;
                }
                self.report_progress(
                    index,
                    Phase::Downloading,
                    progress.fraction(),
                    Some(progress),
                );
            }
            ExportProgressEvent::Conversion(progress) => {
                self.report_progress(index, Phase::Converting, progress, None);
            }
        }
    }

    fn report_progress(
        &mut self,
        index: usize,
        phase: Phase,
        progress: f64,
        transferred: Option<TransferProgress>,
    ) {
        let JobState::Running(current_phase, current) = *self.snapshot.jobs[index].state() else {
            return;
        };
        let is_worth_reporting = progress >= 1.0
            || progress - current >= 0.005
            || (transferred.is_some() && current == 0.0);
        if current_phase == phase && is_worth_reporting {
            let _ = self.snapshot.jobs[index].transition_with(
                JobState::Running(phase, progress.clamp(0.0, 1.0)),
                transferred,
            );
        }
    }
}
