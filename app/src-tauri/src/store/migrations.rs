//! Numbered migrations from `migrations/`, tracked in `PRAGMA user_version`.
//! Append new files here; never edit one that has shipped (AGENTS.md rule 9).

use rusqlite::Connection;

use super::StoreError;

/// `(version, sql)` in ascending order. The version is the file's number.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("../../migrations/0001_init.sql"))];

pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |(version, _)| *version)
}

/// Applies every migration newer than the database, each in its own transaction.
pub fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let latest = latest_version();
    if current > latest {
        return Err(StoreError::TooNew {
            found: current,
            latest,
        });
    }
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "migration applied");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn applies_to_empty_db_and_is_idempotent() {
        let mut conn = fresh();
        assert_eq!(version(&conn), latest_version());
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
    }

    #[test]
    fn creates_every_table_from_the_data_model() {
        let conn = fresh();
        for table in [
            "people",
            "meetings",
            "speakers",
            "segments",
            "reports",
            "action_items",
            "scores",
            "highlights",
            "tags",
            "meeting_tags",
            "jobs",
            "app_rules",
            "settings",
            "segments_fts",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "missing table {table}");
        }
    }

    #[test]
    fn deleting_a_meeting_cascades_to_segments() {
        let conn = fresh();
        conn.execute_batch(
            "INSERT INTO meetings (id, title, source_app, started_at, status, audio_dir, created_at, updated_at)
               VALUES ('m1', 't', 'zoom', 0, 'ready', 'd', 0, 0);
             INSERT INTO segments (id, meeting_id, track, start_ms, end_ms, text, created_at, updated_at)
               VALUES ('s1', 'm1', 'mic', 0, 1, 'hi', 0, 0);
             DELETE FROM meetings WHERE id = 'm1';",
        )
        .unwrap();
        let left: i64 = conn
            .query_row("SELECT count(*) FROM segments", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn rejects_db_from_a_newer_app() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", latest_version() + 1)
            .unwrap();
        assert!(matches!(migrate(&mut conn), Err(StoreError::TooNew { .. })));
    }
}
