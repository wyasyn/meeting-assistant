//! Provider API keys in the OS keychain (FR-8.1, rule 8). Never in SQLite, files or logs.

use super::ProviderId;
use crate::store::StoreError;

/// Where API keys live. Faked in tests.
pub trait ApiKeys: Send + Sync {
    fn get(&self, provider: ProviderId) -> Result<Option<String>, StoreError>;
    fn set(&self, provider: ProviderId, key: &str) -> Result<(), StoreError>;
    /// Removing a key that is not there is not an error.
    fn clear(&self, provider: ProviderId) -> Result<(), StoreError>;
}

/// One keychain entry per provider, under the bundle identifier like the database key.
pub struct KeyringApiKeys {
    service: String,
}

impl KeyringApiKeys {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, provider: ProviderId) -> Result<keyring::Entry, StoreError> {
        Ok(keyring::Entry::new(
            &self.service,
            &format!("api-key:{}", provider.as_str()),
        )?)
    }
}

impl ApiKeys for KeyringApiKeys {
    fn get(&self, provider: ProviderId) -> Result<Option<String>, StoreError> {
        match self.entry(provider)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, provider: ProviderId, key: &str) -> Result<(), StoreError> {
        Ok(self.entry(provider)?.set_password(key)?)
    }

    fn clear(&self, provider: ProviderId) -> Result<(), StoreError> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct MemoryApiKeys(pub Mutex<HashMap<ProviderId, String>>);

    impl ApiKeys for MemoryApiKeys {
        fn get(&self, provider: ProviderId) -> Result<Option<String>, StoreError> {
            Ok(self.0.lock().unwrap().get(&provider).cloned())
        }
        fn set(&self, provider: ProviderId, key: &str) -> Result<(), StoreError> {
            self.0.lock().unwrap().insert(provider, key.to_owned());
            Ok(())
        }
        fn clear(&self, provider: ProviderId) -> Result<(), StoreError> {
            self.0.lock().unwrap().remove(&provider);
            Ok(())
        }
    }
}
