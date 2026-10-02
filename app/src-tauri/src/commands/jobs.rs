//! Job queue events (NFR-7). No commands yet: `reprocess` (FR-4.6) comes with the pipeline.

use tauri::{AppHandle, Emitter, Runtime};

use crate::jobs::{JobEvents, JobProgress};

pub const PROGRESS_EVENT: &str = "job:progress";

/// Sends job progress to the window.
pub struct TauriJobEvents<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> JobEvents for TauriJobEvents<R> {
    fn progress(&self, progress: &JobProgress) {
        if let Err(err) = self.0.emit(PROGRESS_EVENT, progress.clone()) {
            tracing::debug!(%err, "could not emit job:progress");
        }
    }
}
