//! `list_meetings`, `get_meeting`, `rename_speaker`, `get_segment_audio`, `export_meeting`
//! (FR-3.3, FR-3.8, FR-6.1, FR-7.1). Thin: the work is in `crate::meetings`.

use tauri::ipc::Response;
use tauri::State;

use crate::error::AppError;
use crate::meetings::{ExportFormat, MeetingDetail, MeetingListItem, MeetingService, Page};
use crate::store::segments::Speaker;

/// Runs database and decoding work off the main thread.
async fn blocking<T: Send + 'static>(
    service: &MeetingService,
    work: impl FnOnce(&MeetingService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let service = service.clone();
    tauri::async_runtime::spawn_blocking(move || work(&service))
        .await
        .map_err(AppError::internal)?
}

/// The other filters in the contract (`query`, `tag`, `sourceApp`, dates) join in 2.5 and 3.1.
#[tauri::command]
pub async fn list_meetings(
    service: State<'_, MeetingService>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<Page<MeetingListItem>, AppError> {
    blocking(&service, move |s| s.list(cursor.as_deref(), limit)).await
}

#[tauri::command]
pub async fn get_meeting(
    service: State<'_, MeetingService>,
    id: String,
) -> Result<MeetingDetail, AppError> {
    blocking(&service, move |s| s.detail(&id)).await
}

/// `personId` is reserved for FR-3.4.
#[tauri::command]
pub async fn rename_speaker(
    service: State<'_, MeetingService>,
    speaker_id: String,
    name: String,
) -> Result<Speaker, AppError> {
    blocking(&service, move |s| s.rename_speaker(&speaker_id, &name)).await
}

/// A WAV file as raw bytes (an `ArrayBuffer` in the window), never a path (ADR-022).
#[tauri::command]
pub async fn get_segment_audio(
    service: State<'_, MeetingService>,
    segment_id: String,
) -> Result<Response, AppError> {
    let wav = blocking(&service, move |s| s.segment_audio(&segment_id)).await?;
    Ok(Response::new(wav))
}

/// Saves an export the window rendered (`text`) to the path picked in the save dialog.
#[tauri::command]
pub async fn export_meeting(
    service: State<'_, MeetingService>,
    id: String,
    format: ExportFormat,
    path: String,
    text: String,
) -> Result<String, AppError> {
    blocking(&service, move |s| {
        s.export(&id, format, std::path::Path::new(&path), &text)
    })
    .await
}
