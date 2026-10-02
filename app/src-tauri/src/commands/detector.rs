//! Detector events (FR-1.1, FR-1.2). No commands yet: the prompt and its answer are 1.5.

use tauri::{AppHandle, Emitter, Runtime};

use crate::detector::{DetectorEvents, MeetingDetected};

pub const DETECTED_EVENT: &str = "meeting:detected";

/// Sends detections to the window.
pub struct TauriDetectorEvents<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> DetectorEvents for TauriDetectorEvents<R> {
    fn detected(&self, signal: &MeetingDetected) {
        if let Err(err) = self.0.emit(DETECTED_EVENT, signal.clone()) {
            tracing::debug!(%err, "could not emit meeting:detected");
        }
    }
}
