//! `meetings` table: as much as recording and crash recovery need (FR-2.4, NFR-6).

use rusqlite::{params, Connection, OptionalExtension};

use super::{now_ms, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingStatus {
    Recording,
    Processing,
    Ready,
    Failed,
    /// The app stopped while recording; the audio up to the last chunk is kept.
    Interrupted,
}

impl MeetingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Processing => "processing",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

/// A meeting left in `recording` when the app last stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRecording {
    pub id: String,
    /// Relative to the app data dir.
    pub audio_dir: String,
    pub started_at: i64,
}

pub struct MeetingRepo<'a> {
    conn: &'a Connection,
}

impl<'a> MeetingRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Inserts a meeting in `recording` and returns it. Audio goes to `meetings/<id>`.
    pub fn create_recording(
        &self,
        title: &str,
        source_app: &str,
        started_at: i64,
    ) -> Result<OpenRecording, StoreError> {
        let id = uuid::Uuid::now_v7().to_string();
        let audio_dir = format!("meetings/{id}");
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO meetings (id, title, source_app, started_at, status, audio_dir, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, title, source_app, started_at, MeetingStatus::Recording.as_str(), audio_dir, now],
        )?;
        Ok(OpenRecording {
            id,
            audio_dir,
            started_at,
        })
    }

    pub fn finish(
        &self,
        id: &str,
        ended_at: i64,
        duration_s: i64,
        status: MeetingStatus,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE meetings SET ended_at = ?2, duration_s = ?3, status = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, ended_at, duration_s, status.as_str(), now_ms()],
        )?;
        Ok(())
    }

    /// Meetings still marked `recording`. At startup these were cut off by a crash.
    pub fn open_recordings(&self) -> Result<Vec<OpenRecording>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, audio_dir, started_at FROM meetings WHERE status = ?1 ORDER BY started_at",
        )?;
        let rows = stmt.query_map([MeetingStatus::Recording.as_str()], |row| {
            Ok(OpenRecording {
                id: row.get(0)?,
                audio_dir: row.get(1)?,
                started_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_status(&self, id: &str, status: MeetingStatus) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE meetings SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), now_ms()],
        )?;
        Ok(())
    }

    /// Removes a meeting that never got any audio (a start that failed).
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM meetings WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Audio folder (relative to the app data dir) and language of a meeting.
    pub fn processing_info(
        &self,
        id: &str,
    ) -> Result<Option<(String, Option<String>)>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT audio_dir, language FROM meetings WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn status(&self, id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT status FROM meetings WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn nfr_6_create_list_open_and_finish() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = MeetingRepo::new(&conn);

        let a = repo.create_recording("Standup", "zoom", 1_000).unwrap();
        let b = repo.create_recording("Review", "meet", 2_000).unwrap();
        assert_eq!(a.audio_dir, format!("meetings/{}", a.id));
        assert_eq!(uuid::Uuid::parse_str(&a.id).unwrap().get_version_num(), 7);
        assert_eq!(repo.open_recordings().unwrap(), vec![a.clone(), b.clone()]);

        repo.finish(&a.id, 61_000, 60, MeetingStatus::Interrupted)
            .unwrap();
        assert_eq!(repo.open_recordings().unwrap(), vec![b]);
        assert_eq!(repo.status(&a.id).unwrap().as_deref(), Some("interrupted"));
        let (ended_at, duration_s): (i64, i64) = conn
            .query_row(
                "SELECT ended_at, duration_s FROM meetings WHERE id = ?1",
                [&a.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((ended_at, duration_s), (61_000, 60));
        assert_eq!(repo.status("missing").unwrap(), None);
        repo.delete(&a.id).unwrap();
        assert_eq!(repo.status(&a.id).unwrap(), None);
    }
}
