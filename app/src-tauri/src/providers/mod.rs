//! The `Provider` trait and its vendor implementations (gemini/, deepgram/, local/).
//! Jobs and commands depend on this module only; vendor HTTP stays in `<vendor>/` (rule 7).

pub mod analysis;
pub mod gemini;
pub mod keys;
mod service;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
pub use analysis::AnalysisJson;
pub use service::{ProviderEntry, ProviderService, TestResult};

/// A provider the user can pick. Its key lives in the keychain (FR-8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Gemini,
}

impl ProviderId {
    pub const ALL: [Self; 1] = [Self::Gemini];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
        }
    }
}

/// What a provider can do.
#[allow(dead_code, reason = "read by the pipeline from 1.9 on")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub stt: bool,
    pub diarization: bool,
    pub llm: bool,
    pub embeddings: bool,
    pub offline: bool,
}

/// One decrypted temporary Ogg Opus chunk file and where it starts in the meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub path: PathBuf,
    pub start_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscribeRequest {
    /// In time order. Deleted by the caller after the call.
    pub chunks: Vec<AudioChunk>,
    /// BCP-47 code; `None` lets the provider detect it.
    pub language: Option<String>,
    pub diarize: bool,
    /// Names and terms that may appear.
    pub vocabulary: Vec<String>,
}

/// Times are ms from the meeting start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// The provider's label ("Speaker 1"); `None` when not diarized.
    pub speaker_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscribeResult {
    pub segments: Vec<TranscriptSegment>,
}

/// Prompt input for the analysis (docs/06 "Analysis JSON schema").
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzeRequest {
    /// Lines of `[segment_id] [mm:ss] Speaker: text`.
    pub transcript: String,
    /// The deterministic metrics (docs/07).
    pub metrics: serde_json::Value,
    pub template: String,
    pub highlights: Vec<String>,
}

/// Messages are readable and never hold keys, transcript text or paths (rule 8).
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Network, timeout, rate limit or a server error: trying later may work.
    #[error("{0}")]
    Unavailable(String),
    /// The provider refused the key or the request: trying again will not help.
    #[error("{0}")]
    Rejected(String),
    /// The answer did not match the expected shape (rule 6).
    #[error("{0}")]
    InvalidOutput(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("could not read an audio chunk: {0}")]
    Io(#[from] std::io::Error),
}

impl From<ProviderError> for AppError {
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::Unavailable(m) => Self::ProviderUnavailable(m),
            ProviderError::Rejected(m) | ProviderError::Unsupported(m) => Self::ProviderRejected(m),
            ProviderError::InvalidOutput(m) => Self::InvalidLlmOutput(m),
            ProviderError::Io(e) => {
                tracing::debug!(error = %e, "audio chunk read failed");
                Self::Storage("A recording file could not be read. It may be damaged.".into())
            }
        }
    }
}

/// STT and LLM behind one interface (docs/06 "Provider trait").
#[allow(dead_code, reason = "the pipeline steps call it from 1.9 on")]
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    /// Cheap call that proves the key works ("Test key").
    async fn check(&self) -> Result<(), ProviderError>;
    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, ProviderError>;
    async fn analyze(&self, req: AnalyzeRequest) -> Result<AnalysisJson, ProviderError>;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError>;
    /// US dollars at current list prices (FR-8.4).
    fn estimate_cost(&self, audio_seconds: u32, transcript_tokens: u32) -> f64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_keep_their_contract_codes() {
        let cases = [
            (
                ProviderError::Unavailable("x".into()),
                "provider_unavailable",
                true,
            ),
            (
                ProviderError::Rejected("x".into()),
                "provider_rejected",
                false,
            ),
            (
                ProviderError::InvalidOutput("x".into()),
                "invalid_llm_output",
                true,
            ),
            (
                ProviderError::Unsupported("x".into()),
                "provider_rejected",
                false,
            ),
        ];
        for (err, code, retryable) in cases {
            let err = AppError::from(err);
            assert_eq!((err.code(), err.retryable()), (code, retryable));
        }
    }

    #[test]
    fn provider_ids_are_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_value(ProviderId::Gemini).unwrap(), "gemini");
        assert_eq!(
            serde_json::from_value::<ProviderId>("gemini".into()).unwrap(),
            ProviderId::Gemini
        );
    }
}
