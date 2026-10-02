//! `highlights`: moments the user marked while recording (FR-9.1, ADR-029).

use rusqlite::{params, Connection};
use serde::Serialize;

use super::{now_ms, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub id: String,
    pub meeting_id: String,
    /// Recorded time when it was marked, ms from the meeting start.
    pub at_ms: i64,
    pub note: Option<String>,
}

pub struct HighlightRepo<'a> {
    conn: &'a Connection,
}

impl<'a> HighlightRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn add(
        &self,
        meeting_id: &str,
        at_ms: i64,
        note: Option<&str>,
    ) -> Result<Highlight, StoreError> {
        let id = uuid::Uuid::now_v7().to_string();
        self.conn.execute(
            "INSERT INTO highlights (id, meeting_id, at_ms, note, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, meeting_id, at_ms, note, now_ms()],
        )?;
        Ok(Highlight {
            id,
            meeting_id: meeting_id.to_owned(),
            at_ms,
            note: note.map(str::to_owned),
        })
    }

    /// In time order.
    pub fn list(&self, meeting_id: &str) -> Result<Vec<Highlight>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, at_ms, note FROM highlights WHERE meeting_id = ?1
             ORDER BY at_ms, rowid",
        )?;
        let rows = stmt.query_map([meeting_id], |row| {
            Ok(Highlight {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                at_ms: row.get(2)?,
                note: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meetings::MeetingRepo;
    use crate::store::Store;

    #[test]
    fn fr_9_1_highlights_are_kept_in_time_order() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let id = MeetingRepo::new(&conn)
            .create_recording("Standup", "zoom", 0)
            .unwrap()
            .id;
        let repo = HighlightRepo::new(&conn);
        let late = repo.add(&id, 90_000, Some("pricing")).unwrap();
        repo.add(&id, 5_000, None).unwrap();
        let list = repo.list(&id).unwrap();
        let got: Vec<_> = list.iter().map(|h| (h.at_ms, h.note.as_deref())).collect();
        assert_eq!(got, [(5_000, None), (90_000, Some("pricing"))]);
        assert_eq!(list[1], late);
        assert!(repo.list("other").unwrap().is_empty());
        let wire = serde_json::to_value(&late).unwrap();
        assert_eq!(wire["atMs"], 90_000);
        assert_eq!(wire["meetingId"], serde_json::json!(id));
    }
}
