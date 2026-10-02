//! `ProviderService`: keys in the keychain, building providers from them, "Test key" (FR-8.1).

use std::sync::Arc;

use serde::Serialize;

use super::keys::ApiKeys;
use super::{gemini, Provider, ProviderError, ProviderId};
use crate::error::AppError;

/// Builds a provider from its key. Faked in tests.
pub type Factory =
    Box<dyn Fn(ProviderId, String) -> Result<Arc<dyn Provider>, ProviderError> + Send + Sync>;

/// Mirrors `ProviderEntry` in `app/src/lib/ipc.ts`. The key itself never leaves the keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEntry {
    pub provider: ProviderId,
    pub has_key: bool,
}

/// `test_provider` result: a refused key is an answer, not an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

/// Managed as Tauri state. Cheap to clone.
#[derive(Clone)]
pub struct ProviderService {
    keys: Arc<dyn ApiKeys>,
    make: Arc<Factory>,
}

impl ProviderService {
    pub fn new(keys: Arc<dyn ApiKeys>, make: Factory) -> Self {
        Self {
            keys,
            make: Arc::new(make),
        }
    }

    /// The real vendor implementations.
    pub fn default_factory() -> Factory {
        Box::new(|id, key| match id {
            ProviderId::Gemini => Ok(Arc::new(gemini::GeminiProvider::new(key)?)),
        })
    }

    pub fn list(&self) -> Result<Vec<ProviderEntry>, AppError> {
        ProviderId::ALL
            .into_iter()
            .map(|provider| {
                Ok(ProviderEntry {
                    provider,
                    has_key: self.keys.get(provider)?.is_some(),
                })
            })
            .collect()
    }

    pub fn set_key(&self, provider: ProviderId, key: &str) -> Result<(), AppError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(AppError::InvalidState("Enter an API key first.".into()));
        }
        // Keys go into an HTTP header, so anything else is a paste mistake.
        if !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err(AppError::ProviderRejected(
                "This does not look like an API key. Copy it again.".into(),
            ));
        }
        self.keys.set(provider, key)?;
        tracing::info!(provider = provider.as_str(), "api key saved");
        Ok(())
    }

    pub fn clear_key(&self, provider: ProviderId) -> Result<(), AppError> {
        self.keys.clear(provider)?;
        tracing::info!(provider = provider.as_str(), "api key removed");
        Ok(())
    }

    /// The provider built with its saved key.
    pub fn get(&self, provider: ProviderId) -> Result<Arc<dyn Provider>, AppError> {
        let key = self.keys.get(provider)?.ok_or_else(|| no_key(provider))?;
        Ok((self.make)(provider, key)?)
    }

    /// Asks the provider whether it accepts the saved key. Keychain failures are errors;
    /// anything the provider says is the answer.
    pub async fn test(&self, provider: ProviderId) -> Result<TestResult, AppError> {
        let Some(key) = self.keys.get(provider)? else {
            return Ok(TestResult {
                ok: false,
                message: no_key(provider).to_string(),
            });
        };
        let result = match (self.make)(provider, key) {
            Ok(p) => p.check().await,
            Err(e) => Err(e),
        };
        tracing::info!(
            provider = provider.as_str(),
            ok = result.is_ok(),
            "api key tested"
        );
        Ok(match result {
            Ok(()) => TestResult {
                ok: true,
                message: format!("{} accepted the key.", provider.label()),
            },
            Err(e) => TestResult {
                ok: false,
                message: e.to_string(),
            },
        })
    }
}

fn no_key(provider: ProviderId) -> AppError {
    AppError::NoApiKey(format!(
        "No {} API key is saved. Add one in settings.",
        provider.label()
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::super::keys::tests::MemoryApiKeys;
    use super::super::{
        AnalysisJson, AnalyzeRequest, Capabilities, TranscribeRequest, TranscribeResult,
    };
    use super::*;

    /// Accepts only the key "good"; "down" means the provider cannot be reached.
    struct FakeProvider(String);

    #[async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Gemini
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                stt: true,
                diarization: true,
                llm: true,
                embeddings: false,
                offline: false,
            }
        }
        async fn check(&self) -> Result<(), ProviderError> {
            match self.0.as_str() {
                "good" => Ok(()),
                "down" => Err(ProviderError::Unavailable("Could not reach Gemini.".into())),
                _ => Err(ProviderError::Rejected("Gemini refused this key.".into())),
            }
        }
        async fn transcribe(
            &self,
            _: TranscribeRequest,
        ) -> Result<TranscribeResult, ProviderError> {
            Ok(TranscribeResult::default())
        }
        async fn analyze(&self, _: AnalyzeRequest) -> Result<AnalysisJson, ProviderError> {
            Err(ProviderError::Unsupported("no".into()))
        }
        async fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
            Ok(Vec::new())
        }
        fn estimate_cost(&self, _: u32, _: u32) -> f64 {
            0.0
        }
    }

    fn service() -> (ProviderService, Arc<MemoryApiKeys>, Arc<Mutex<Vec<String>>>) {
        let keys = Arc::new(MemoryApiKeys::default());
        let built = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&built);
        let service = ProviderService::new(
            Arc::clone(&keys) as Arc<dyn ApiKeys>,
            Box::new(move |_, key| {
                seen.lock().unwrap().push(key.clone());
                Ok(Arc::new(FakeProvider(key)) as Arc<dyn Provider>)
            }),
        );
        (service, keys, built)
    }

    fn test(service: &ProviderService) -> TestResult {
        tauri::async_runtime::block_on(service.test(ProviderId::Gemini)).unwrap()
    }

    #[test]
    fn fr_8_1_keys_are_saved_trimmed_listed_and_removed() {
        let (service, keys, _) = service();
        let has_key = |s: &ProviderService| s.list().unwrap()[0].has_key;
        assert!(!has_key(&service));

        service.set_key(ProviderId::Gemini, "  good\n").unwrap();
        assert_eq!(
            keys.get(ProviderId::Gemini).unwrap().as_deref(),
            Some("good")
        );
        assert!(has_key(&service));

        service.clear_key(ProviderId::Gemini).unwrap();
        assert!(!has_key(&service));
        service.clear_key(ProviderId::Gemini).unwrap();
    }

    #[test]
    fn fr_8_1_blank_or_mangled_keys_are_not_saved() {
        let (service, keys, _) = service();
        let blank = service.set_key(ProviderId::Gemini, "   ").unwrap_err();
        assert_eq!(blank.code(), "invalid_state");
        let mangled = service.set_key(ProviderId::Gemini, "abc def").unwrap_err();
        assert_eq!(mangled.code(), "provider_rejected");
        assert_eq!(keys.get(ProviderId::Gemini).unwrap(), None);
    }

    #[test]
    fn fr_8_1_test_key_reports_what_the_provider_said() {
        let (service, _, built) = service();
        let missing = test(&service);
        assert!(!missing.ok);
        assert!(missing.message.contains("No Gemini API key"));
        assert!(built.lock().unwrap().is_empty());

        service.set_key(ProviderId::Gemini, "good").unwrap();
        assert_eq!(
            test(&service),
            TestResult {
                ok: true,
                message: "Gemini accepted the key.".into()
            }
        );
        service.set_key(ProviderId::Gemini, "bad").unwrap();
        assert_eq!(test(&service).message, "Gemini refused this key.");
        service.set_key(ProviderId::Gemini, "down").unwrap();
        assert!(!test(&service).ok);
    }

    #[test]
    fn a_provider_needs_a_saved_key() {
        let (service, _, built) = service();
        let err = service.get(ProviderId::Gemini).err().unwrap();
        assert_eq!(err.code(), "no_api_key");
        service.set_key(ProviderId::Gemini, "good").unwrap();
        assert_eq!(
            service.get(ProviderId::Gemini).unwrap().id(),
            ProviderId::Gemini
        );
        assert_eq!(*built.lock().unwrap(), ["good"]);
    }

    #[test]
    fn wire_shapes_are_camel_case() {
        let entry = ProviderEntry {
            provider: ProviderId::Gemini,
            has_key: true,
        };
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::json!({ "provider": "gemini", "hasKey": true })
        );
    }
}
