//! `AppError`: the one error type commands return to the UI.
//! Serialises to `{ code, message, retryable }` (docs/06-api-contracts.md, "Errors").

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Message shown for failures that have no contract code. The detail goes to the log.
pub const INTERNAL_MESSAGE: &str = "Something went wrong. Details are in the log file.";

/// Every variant carries a readable message for the UI. Never put API keys,
/// transcript text or audio paths in it.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    NoApiKey(String),
    #[error("{0}")]
    ProviderUnavailable(String),
    #[error("{0}")]
    ProviderRejected(String),
    #[error("{0}")]
    AudioDevice(String),
    #[error("{0}")]
    PermissionDenied(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    InvalidLlmOutput(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    SidecarDown(String),
    #[error("{INTERNAL_MESSAGE}")]
    Internal,
}

impl AppError {
    /// Wraps an unexpected failure. The detail may hold paths, so it is logged at debug only.
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        tracing::debug!(%detail, "internal error");
        Self::Internal
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NoApiKey(_) => "no_api_key",
            Self::ProviderUnavailable(_) => "provider_unavailable",
            Self::ProviderRejected(_) => "provider_rejected",
            Self::AudioDevice(_) => "audio_device",
            Self::PermissionDenied(_) => "permission_denied",
            Self::Storage(_) => "storage",
            Self::InvalidLlmOutput(_) => "invalid_llm_output",
            Self::NotFound(_) => "not_found",
            Self::SidecarDown(_) => "sidecar_down",
            Self::Internal => "internal",
        }
    }

    /// True when trying the same action again later may succeed.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::ProviderUnavailable(_) | Self::InvalidLlmOutput(_) | Self::SidecarDown(_)
        )
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 3)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.serialize_field("retryable", &self.retryable())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialises_to_contract_shape() {
        let msg = || "msg".to_string();
        let cases = [
            (AppError::NoApiKey(msg()), "no_api_key", false),
            (
                AppError::ProviderUnavailable(msg()),
                "provider_unavailable",
                true,
            ),
            (
                AppError::ProviderRejected(msg()),
                "provider_rejected",
                false,
            ),
            (AppError::AudioDevice(msg()), "audio_device", false),
            (
                AppError::PermissionDenied(msg()),
                "permission_denied",
                false,
            ),
            (AppError::Storage(msg()), "storage", false),
            (
                AppError::InvalidLlmOutput(msg()),
                "invalid_llm_output",
                true,
            ),
            (AppError::NotFound(msg()), "not_found", false),
            (AppError::SidecarDown(msg()), "sidecar_down", true),
        ];
        for (error, code, retryable) in cases {
            let value = serde_json::to_value(&error).unwrap();
            assert_eq!(
                value,
                json!({ "code": code, "message": "msg", "retryable": retryable }),
                "{code}"
            );
        }
    }

    #[test]
    fn internal_hides_detail() {
        let error = AppError::internal("/home/user/secret/path.opus: disk full");
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(
            value,
            json!({ "code": "internal", "message": INTERNAL_MESSAGE, "retryable": false })
        );
    }
}
