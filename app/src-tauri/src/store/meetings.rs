//! `meetings` table: recording, crash recovery (FR-2.4, NFR-6) and reading meetings back (1.10).

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

/// One `meetings` row as the library and transcript view read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRow {
    pub id: String,
    pub title: String,
    pub source_app: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_s: Option<i64>,
    pub status: String,
    /// Relative to the app data dir.
    pub audio_dir: String,
    pub audio_deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisInfo {
    pub duration_s: Option<i64>,
    /// From the calendar, for overrun.
    pub scheduled_duration_s: Option<i64>,
    pub template: String,
}

const ROW_COLUMNS: &str =
    "id, title, source_app, started_at, ended_at, duration_s, status, audio_dir, audio_deleted_at";

fn meeting_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingRow> {
    Ok(MeetingRow {
        id: row.get(0)?,
        title: row.get(1)?,
        source_app: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        duration_s: row.get(5)?,
        status: row.get(6)?,
        audio_dir: row.get(7)?,
        audio_deleted_at: row.get(8)?,
    })
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

    /// What the metrics and analyze steps need besides the transcript.
    pub fn analysis_info(&self, id: &str) -> Result<Option<AnalysisInfo>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT duration_s, scheduled_duration_s, template FROM meetings WHERE id = ?1",
                [id],
                |row| {
                    Ok(AnalysisInfo {
                        duration_s: row.get(0)?,
                        scheduled_duration_s: row.get(1)?,
                        template: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn get(&self, id: &str) -> Result<Option<MeetingRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ROW_COLUMNS} FROM meetings WHERE id = ?1"),
                [id],
                meeting_row,
            )
            .optional()?)
    }

    /// Newest first. `before` is the `started_at` of the last meeting of the previous page;
    /// UUID v7 ids break ties, so the cursor is `(started_at, id)`.
    pub fn list(
        &self,
        before: Option<(i64, &str)>,
        limit: u32,
    ) -> Result<Vec<MeetingRow>, StoreError> {
        let (started_at, id) = before.unwrap_or((i64::MAX, ""));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM meetings
             WHERE started_at < ?1 OR (started_at = ?1 AND id < ?2)
             ORDER BY started_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![started_at, id, limit], meeting_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
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

    #[test]
    fn fr_6_1_lists_newest_first_in_pages() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = MeetingRepo::new(&conn);
        let a = repo.create_recording("A", "zoom", 1_000).unwrap();
        let b = repo.create_recording("B", "zoom", 2_000).unwrap();
        let c = repo.create_recording("C", "zoom", 2_000).unwrap();
        repo.finish(&a.id, 61_000, 60, MeetingStatus::Ready)
            .unwrap();

        let titles = |rows: Vec<MeetingRow>| rows.into_iter().map(|r| r.title).collect::<Vec<_>>();
        let page = repo.list(None, 2).unwrap();
        let last = page.last().unwrap().clone();
        assert_eq!(titles(page), ["C", "B"]);
        let next = repo.list(Some((last.started_at, &last.id)), 2).unwrap();
        assert_eq!(titles(next), ["A"]);

        let row = repo.get(&a.id).unwrap().unwrap();
        assert_eq!(
            row,
            MeetingRow {
                id: a.id.clone(),
                title: "A".into(),
                source_app: "zoom".into(),
                started_at: 1_000,
                ended_at: Some(61_000),
                duration_s: Some(60),
                status: "ready".into(),
                audio_dir: a.audio_dir,
                audio_deleted_at: None,
            }
        );
        assert_eq!(repo.get("missing").unwrap(), None);
        assert!(b.id < c.id);
    }
}
