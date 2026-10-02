//! `settings` table: plain key/value app settings. Never API keys (those live in the keychain).

use rusqlite::{params, Connection, OptionalExtension};

use super::{now_ms, StoreError};

pub struct SettingsRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SettingsRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now_ms()],
        )?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn set_get_overwrite_delete() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = SettingsRepo::new(&conn);

        assert_eq!(repo.get("theme").unwrap(), None);
        repo.set("theme", "dark").unwrap();
        assert_eq!(repo.get("theme").unwrap().as_deref(), Some("dark"));

        conn.execute("UPDATE settings SET updated_at = 0", [])
            .unwrap();
        repo.set("theme", "light").unwrap();
        assert_eq!(repo.get("theme").unwrap().as_deref(), Some("light"));
        let updated_at: i64 = conn
            .query_row(
                "SELECT updated_at FROM settings WHERE key = 'theme'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(updated_at > 0);

        repo.delete("theme").unwrap();
        assert_eq!(repo.get("theme").unwrap(), None);
    }
}
