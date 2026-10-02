//! The database key: 32 random bytes kept in the OS keychain (NFR-13).
//! The key is never logged and never put in an error message.

use super::StoreError;

pub const KEY_LEN: usize = 32;
const KEYRING_ACCOUNT: &str = "db-key";

/// Where the database key lives. Faked in tests.
pub trait KeyStore {
    fn get(&self) -> Result<Option<Vec<u8>>, StoreError>;
    fn set(&self, key: &[u8]) -> Result<(), StoreError>;
}

/// OS keychain (Secret Service, Keychain, Credential Manager).
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    /// `service` is the bundle identifier, so each app build gets its own entry.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, StoreError> {
        Ok(keyring::Entry::new(&self.service, KEYRING_ACCOUNT)?)
    }
}

impl KeyStore for KeyringStore {
    fn get(&self) -> Result<Option<Vec<u8>>, StoreError> {
        match self.entry()?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &[u8]) -> Result<(), StoreError> {
        Ok(self.entry()?.set_secret(key)?)
    }
}

/// Returns the stored key, or creates one when the database does not exist yet.
/// A missing key for an existing database is an error: a new key could never open it.
pub fn load_or_create_key(store: &dyn KeyStore, db_exists: bool) -> Result<Vec<u8>, StoreError> {
    match store.get()? {
        Some(key) if key.len() == KEY_LEN => Ok(key),
        Some(_) => Err(StoreError::BadKey),
        None if db_exists => Err(StoreError::KeyMissing),
        None => {
            let mut key = vec![0u8; KEY_LEN];
            getrandom::fill(&mut key).map_err(|e| StoreError::Random(e.to_string()))?;
            store.set(&key)?;
            tracing::info!("database key created in keychain");
            Ok(key)
        }
    }
}

/// Hex form for `PRAGMA key = "x'...'"` (raw key, no passphrase derivation).
pub fn to_hex(key: &[u8]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    pub struct MemoryKeyStore(pub RefCell<Option<Vec<u8>>>);

    impl KeyStore for MemoryKeyStore {
        fn get(&self) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.0.borrow().clone())
        }
        fn set(&self, key: &[u8]) -> Result<(), StoreError> {
            *self.0.borrow_mut() = Some(key.to_vec());
            Ok(())
        }
    }

    #[test]
    fn creates_and_saves_a_key_on_first_run() {
        let store = MemoryKeyStore::default();
        let key = load_or_create_key(&store, false).unwrap();
        assert_eq!(key.len(), KEY_LEN);
        assert_eq!(store.get().unwrap(), Some(key.clone()));
        assert_eq!(load_or_create_key(&store, true).unwrap(), key);
    }

    #[test]
    fn refuses_to_replace_a_lost_key_for_an_existing_db() {
        let store = MemoryKeyStore::default();
        assert!(matches!(
            load_or_create_key(&store, true),
            Err(StoreError::KeyMissing)
        ));
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn rejects_a_key_of_the_wrong_length() {
        let store = MemoryKeyStore(RefCell::new(Some(vec![1, 2, 3])));
        assert!(matches!(
            load_or_create_key(&store, true),
            Err(StoreError::BadKey)
        ));
    }

    #[test]
    fn hex_is_lowercase_two_digits_per_byte() {
        assert_eq!(to_hex(&[0, 15, 255]), "000fff");
    }
}
