//! Consent prompt and app rules (FR-1.3, FR-1.7). Thin: the work is in `crate::consent`.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::consent::{AppRuleEntry, ConsentEvents, ConsentService};
use crate::detector::apps::SourceApp;
use crate::detector::MeetingDetected;
use crate::error::AppError;
use crate::store::app_rules::AppRule;
use crate::tray;

pub const DETECTED_EVENT: &str = "meeting:detected";
pub const PROMPT_CLOSED_EVENT: &str = "meeting:prompt-closed";

/// Payload of `meeting:prompt-closed`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptClosed {
    signal_id: String,
}

/// Sends prompts to the window.
pub struct TauriConsentEvents<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> TauriConsentEvents<R> {
    fn emit<T: Serialize + Clone>(&self, event: &str, payload: T) {
        if let Err(err) = self.0.emit(event, payload) {
            tracing::debug!(%err, event, "could not emit event");
        }
    }
}

impl<R: Runtime> ConsentEvents for TauriConsentEvents<R> {
    fn prompt(&self, signal: &MeetingDetected) {
        self.emit(DETECTED_EVENT, signal.clone());
    }

    fn prompt_closed(&self, signal_id: &str) {
        self.emit(
            PROMPT_CLOSED_EVENT,
            PromptClosed {
                signal_id: signal_id.into(),
            },
        );
    }

    fn show_window(&self) {
        tray::show_main_window(&self.0);
    }
}

#[tauri::command]
pub fn list_app_rules(service: State<'_, ConsentService>) -> Result<Vec<AppRuleEntry>, AppError> {
    service.rules()
}

#[tauri::command]
pub fn set_app_rule(
    service: State<'_, ConsentService>,
    source_app: SourceApp,
    rule: AppRule,
) -> Result<(), AppError> {
    service.set_rule(source_app, rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_closed_uses_camel_case_on_the_wire() {
        let json = serde_json::to_value(PromptClosed {
            signal_id: "s1".into(),
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "signalId": "s1" }));
    }
}
