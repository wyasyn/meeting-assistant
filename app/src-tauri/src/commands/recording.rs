//! Recording commands (FR-1.4, FR-2.3). Thin: the work is in `crate::recording`.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::capture::recorder::Levels;
use crate::capture::{default_backend, AudioDevice};
use crate::consent::ConsentService;
use crate::detector::end::EndReason;
use crate::detector::MeetingEnded;
use crate::error::AppError;
use crate::jobs::JobService;
use crate::recording::{MeetingSummary, Phase, RecordingEvents, RecordingService, RecordingState};
use crate::tray;

pub const STATE_EVENT: &str = "recording:state";
pub const LEVELS_EVENT: &str = "recording:levels";

/// Sends state changes to the window, the tray and consent, and levels to the window.
pub struct TauriEvents<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> TauriEvents<R> {
    fn emit<T: Serialize + Clone>(&self, event: &str, payload: T) {
        if let Err(err) = self.0.emit(event, payload) {
            tracing::debug!(%err, event, "could not emit event");
        }
    }
}

impl<R: Runtime> RecordingEvents for TauriEvents<R> {
    fn state(&self, state: &RecordingState) {
        tray::show_recording_state(&self.0, state.state);
        if let Some(consent) = self.0.try_state::<ConsentService>() {
            consent.recording_changed(state.state);
        }
        // A saved recording waits in `processing`; the queue picks it up now.
        if state.state == Phase::Stopped {
            if let Some(jobs) = self.0.try_state::<JobService>() {
                jobs.wake();
            }
        }
        self.emit(STATE_EVENT, state.clone());
    }

    fn levels(&self, levels: Levels) {
        self.emit(LEVELS_EVENT, levels);
    }

    fn silent(&self, meeting_id: &str) {
        let app = self.0.clone();
        let ended = MeetingEnded {
            meeting_id: meeting_id.into(),
            source_app: None,
            reason: EndReason::Silence,
        };
        // Off the recorder thread: consent reads the recording state, whose lock a
        // pause may hold while it waits for this thread.
        tauri::async_runtime::spawn_blocking(move || {
            if let Some(consent) = app.try_state::<ConsentService>() {
                consent.meeting_ended(&ended);
            }
        });
    }
}

/// Runs blocking service work (device setup, flushing files) off the main thread.
async fn blocking<T: Send + 'static>(
    service: &RecordingService,
    work: impl FnOnce(&RecordingService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let service = service.clone();
    tauri::async_runtime::spawn_blocking(move || work(&service))
        .await
        .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn start_recording(
    service: State<'_, RecordingService>,
    title: Option<String>,
    source_app: Option<String>,
) -> Result<MeetingSummary, AppError> {
    blocking(&service, move |s| s.start(title, source_app)).await
}

#[tauri::command]
pub async fn pause_recording(
    service: State<'_, RecordingService>,
) -> Result<RecordingState, AppError> {
    blocking(&service, RecordingService::pause).await
}

#[tauri::command]
pub async fn resume_recording(
    service: State<'_, RecordingService>,
) -> Result<RecordingState, AppError> {
    blocking(&service, RecordingService::resume).await
}

#[tauri::command]
pub async fn stop_recording(
    service: State<'_, RecordingService>,
) -> Result<RecordingState, AppError> {
    blocking(&service, RecordingService::stop).await
}

#[tauri::command]
pub fn get_recording_state(
    service: State<'_, RecordingService>,
) -> Result<RecordingState, AppError> {
    service.state()
}

#[tauri::command]
pub async fn list_audio_devices() -> Result<Vec<AudioDevice>, AppError> {
    // Listing does two PipeWire roundtrips; keep it off the main thread.
    tauri::async_runtime::spawn_blocking(|| default_backend().list_devices())
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::from)
}
