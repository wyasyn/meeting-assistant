//! SQLite repo layer, migrations and encryption.
//! One SQLCipher file at `<app_data>/db/app.sqlite`, keyed from the OS keychain (NFR-13).

pub mod app_rules;
pub mod crypto;
pub mod highlights;
pub mod jobs;
pub mod key;
pub mod meetings;
mod migrations;
pub mod reports;
pub mod segments;
#[allow(dead_code, reason = "first caller arrives with the FR-8.3 settings")]
pub mod settings;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode};

use crate::error::AppError;
use crypto::AudioKey;
use key::KeyStore;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("keychain: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("random source: {0}")]
    Random(String),
    #[error("database exists but its key is missing from the keychain")]
    KeyMissing,
    #[error("key in the keychain has the wrong length")]
    BadKey,
    #[error("key does not unlock the database")]
    WrongKey,
    #[error("database version {found} is newer than this app supports ({latest})")]
    TooNew { found: u32, latest: u32 },
    #[error("database lock poisoned")]
    Poisoned,
    #[error("audio chunk could not be encrypted or decrypted")]
    Crypto,
}

impl From<StoreError> for AppError {
    fn from(err: StoreError) -> Self {
        tracing::debug!(error = %err, "store error");
        let message = match err {
            StoreError::Keyring(_) => {
                "Could not reach the system keychain. Check that a keyring service is running."
            }
            StoreError::KeyMissing | StoreError::BadKey | StoreError::WrongKey => {
                "The meetings database could not be unlocked with the key in the system keychain."
            }
            StoreError::TooNew { .. } => {
                "The meetings database was saved by a newer version of the app. Please update."
            }
            StoreError::Crypto => "A recording file could not be read. It may be damaged.",
            _ => "Could not read or write the meetings database.",
        };
        AppError::Storage(message.to_owned())
    }
}

/// Epoch milliseconds, UTC.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The open, migrated database. Managed as Tauri state.
pub struct Store {
    conn: Mutex<Connection>,
    audio_key: AudioKey,
}

impl Store {
    /// Opens (or creates) the encrypted database at `path` and runs pending migrations.
    pub fn open(path: &Path, keys: &dyn KeyStore) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let key = key::load_or_create_key(keys, path.exists())?;
        let conn = Connection::open(path)?;
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key::to_hex(&key)))?;
        // The first read fails with NOTADB when the key is wrong.
        match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        }) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == ErrorCode::NotADatabase => {
                return Err(StoreError::WrongKey)
            }
            Err(e) => return Err(e.into()),
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self::init(conn, AudioKey::new(&key)?)?;
        tracing::info!("store opened");
        Ok(store)
    }

    /// Unencrypted in-memory database with the full schema, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(
            Connection::open_in_memory()?,
            AudioKey::new(&[0x11; key::KEY_LEN])?,
        )
    }

    fn init(mut conn: Connection, audio_key: AudioKey) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            audio_key,
        })
    }

    /// Key for audio chunk files; the same key as the database (docs/04).
    pub fn audio_key(&self) -> &AudioKey {
        &self.audio_key
    }

    /// Locks the connection. Repos borrow it: `SettingsRepo::new(&store.conn()?)`.
    pub fn conn(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::key::tests::MemoryKeyStore;
    use super::settings::SettingsRepo;
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ma-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn encrypted_db_reopens_with_its_key_and_not_another() {
        let dir = TempDir::new("reopen");
        let path = dir.0.join("db").join("app.sqlite");
        let keys = MemoryKeyStore::default();

        let store = Store::open(&path, &keys).unwrap();
        SettingsRepo::new(&store.conn().unwrap())
            .set("k", "v")
            .unwrap();
        drop(store);

        let header = std::fs::read(&path).unwrap();
        assert!(
            !header.starts_with(b"SQLite format 3"),
            "file is not encrypted"
        );

        let store = Store::open(&path, &keys).unwrap();
        let value = SettingsRepo::new(&store.conn().unwrap()).get("k").unwrap();
        assert_eq!(value.as_deref(), Some("v"));
        drop(store);

        let other = MemoryKeyStore(RefCell::new(Some(vec![7; key::KEY_LEN])));
        assert!(matches!(
            Store::open(&path, &other),
            Err(StoreError::WrongKey)
        ));
    }

    #[test]
    fn store_errors_become_readable_storage_errors() {
        let err: AppError = StoreError::WrongKey.into();
        assert_eq!(err.code(), "storage");
        assert!(!err.to_string().contains("key does not unlock"));
    }
}
