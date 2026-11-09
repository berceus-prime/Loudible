use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{ExporterError, Phase, TransferProgress};

/// Progress events carry identity so delayed adapter callbacks cannot affect later transfers.
#[derive(Clone, Copy, Debug)]
pub enum ExportProgressEvent {
    Phase(Phase),
    BeginDownload(Uuid),
    Download(Uuid, TransferProgress),
    EndDownload(Uuid),
    Conversion(f64),
}

/// Application callbacks needed by a workflow, independent of any queue implementation.
#[derive(Clone)]
pub struct ExportExecutionContext {
    pub report: Arc<dyn Fn(ExportProgressEvent) + Send + Sync>,
    pub cancel: CancellationToken,
}

impl ExportExecutionContext {
    pub fn phase(&self, phase: Phase) {
        (self.report)(ExportProgressEvent::Phase(phase));
    }

    pub fn check_cancellation(&self) -> Result<(), ExporterError> {
        if self.cancel.is_cancelled() {
            Err(ExporterError::Cancelled)
        } else {
            Ok(())
        }
    }
}
