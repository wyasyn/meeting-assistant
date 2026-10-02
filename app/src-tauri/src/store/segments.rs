//! `segments` and `speakers`: the transcript (FR-3.1, FR-3.2, FR-2.2, ADR-021).
//! Each track is written whole by its transcribe step, so a rerun replaces it.

use rusqlite::{params, Connection};

use super::{now_ms, StoreError};
use crate::capture::Track;

/// Label of the one speaker on the mic track (rule 4).
pub const ME_LABEL: &str = "Me";

/// A transcribed segment before it is stored. Times are ms from the meeting start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// Diarization label on the system track; ignored on the mic track.
    pub speaker_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub id: String,
    pub track: String,
    pub speaker_id: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[allow(dead_code, reason = "read by the transcript view (1.10)")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    pub id: String,
    pub label: String,
    pub is_me: bool,
}

pub struct SegmentRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SegmentRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replaces one track's segments and speakers in one transaction. The mic track gets
    /// one speaker, "Me" (`is_me`); the system track one per label, in order of first
    /// appearance, and no speaker for unlabelled segments.
    pub fn replace_track(
        &self,
        meeting_id: &str,
        track: Track,
        segments: &[NewSegment],
    ) -> Result<(), StoreError> {
        let is_me = track == Track::Mic;
        let now = now_ms();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM segments WHERE meeting_id = ?1 AND track = ?2",
            params![meeting_id, track.as_str()],
        )?;
        tx.execute(
            "DELETE FROM speakers WHERE meeting_id = ?1 AND is_me = ?2",
            params![meeting_id, is_me],
        )?;
        let mut speakers: Vec<(String, String)> = Vec::new();
        let mut speaker_for = |label: Option<&str>| -> Result<Option<String>, StoreError> {
            let label = match (is_me, label) {
                (true, _) => ME_LABEL,
                (false, Some(label)) => label,
                (false, None) => return Ok(None),
            };
            if let Some((_, id)) = speakers.iter().find(|(l, _)| l == label) {
                return Ok(Some(id.clone()));
            }
            let id = uuid::Uuid::now_v7().to_string();
            tx.execute(
                "INSERT INTO speakers (id, meeting_id, label, is_me, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id, meeting_id, label, is_me, now],
            )?;
            speakers.push((label.to_owned(), id.clone()));
            Ok(Some(id))
        };
        for seg in segments {
            let speaker_id = speaker_for(seg.speaker_label.as_deref())?;
            tx.execute(
                "INSERT INTO segments (id, meeting_id, speaker_id, track, start_ms, end_ms, text, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    meeting_id,
                    speaker_id,
                    track.as_str(),
                    seg.start_ms,
                    seg.end_ms,
                    seg.text,
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Both tracks, in time order.
    pub fn list(&self, meeting_id: &str) -> Result<Vec<Segment>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track, speaker_id, start_ms, end_ms, text FROM segments
             WHERE meeting_id = ?1 ORDER BY start_ms, rowid",
        )?;
        let rows = stmt.query_map([meeting_id], |row| {
            Ok(Segment {
                id: row.get(0)?,
                track: row.get(1)?,
                speaker_id: row.get(2)?,
                start_ms: row.get(3)?,
                end_ms: row.get(4)?,
                text: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    #[allow(dead_code, reason = "read by the transcript view (1.10)")]
    pub fn speakers(&self, meeting_id: &str) -> Result<Vec<Speaker>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, is_me FROM speakers WHERE meeting_id = ?1 ORDER BY is_me DESC, rowid",
        )?;
        let rows = stmt.query_map([meeting_id], |row| {
            Ok(Speaker {
                id: row.get(0)?,
                label: row.get(1)?,
                is_me: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn delete(&self, ids: &[String]) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        for id in ids {
            tx.execute("DELETE FROM segments WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meetings::MeetingRepo;
    use crate::store::Store;

    fn seg(start_ms: i64, text: &str, speaker: Option<&str>) -> NewSegment {
        NewSegment {
            start_ms,
            end_ms: start_ms + 500,
            text: text.into(),
            speaker_label: speaker.map(Into::into),
        }
    }

    #[test]
    fn fr_2_2_mic_is_me_and_fr_3_2_system_speakers_follow_their_labels() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let m = MeetingRepo::new(&conn)
            .create_recording("Standup", "zoom", 0)
            .unwrap();
        let repo = SegmentRepo::new(&conn);
        repo.replace_track(
            &m.id,
            Track::Mic,
            &[seg(1_000, "Hi all", Some("Speaker 9"))],
        )
        .unwrap();
        repo.replace_track(
            &m.id,
            Track::Sys,
            &[
                seg(3_000, "Morning", Some("Speaker 2")),
                seg(500, "Hello", Some("Speaker 1")),
                seg(4_000, "Hm", None),
                seg(5_000, "Yes", Some("Speaker 2")),
            ],
        )
        .unwrap();

        let speakers = repo.speakers(&m.id).unwrap();
        let labels: Vec<_> = speakers
            .iter()
            .map(|s| (s.label.as_str(), s.is_me))
            .collect();
        assert_eq!(
            labels,
            [("Me", true), ("Speaker 2", false), ("Speaker 1", false)]
        );
        let label_of = |id: &Option<String>| {
            id.as_ref()
                .map(|id| speakers.iter().find(|s| &s.id == id).unwrap().label.clone())
        };
        let got: Vec<_> = repo
            .list(&m.id)
            .unwrap()
            .iter()
            .map(|s| (s.start_ms, s.track.clone(), label_of(&s.speaker_id)))
            .collect();
        let row =
            |ms, track: &str, label: Option<&str>| (ms, track.to_owned(), label.map(Into::into));
        assert_eq!(
            got,
            [
                row(500, "sys", Some("Speaker 1")),
                row(1_000, "mic", Some("Me")),
                row(3_000, "sys", Some("Speaker 2")),
                row(4_000, "sys", None),
                row(5_000, "sys", Some("Speaker 2")),
            ]
        );
    }

    #[test]
    fn a_rerun_replaces_only_its_own_track() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let m = MeetingRepo::new(&conn)
            .create_recording("Standup", "zoom", 0)
            .unwrap();
        let repo = SegmentRepo::new(&conn);
        repo.replace_track(&m.id, Track::Mic, &[seg(0, "One", None)])
            .unwrap();
        repo.replace_track(&m.id, Track::Sys, &[seg(0, "Two", Some("Speaker 1"))])
            .unwrap();
        repo.replace_track(&m.id, Track::Sys, &[seg(0, "Three", Some("Speaker 1"))])
            .unwrap();
        repo.replace_track(&m.id, Track::Mic, &[]).unwrap();

        let texts: Vec<_> = repo
            .list(&m.id)
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["Three"]);
        assert_eq!(repo.speakers(&m.id).unwrap().len(), 1);

        let id = repo.list(&m.id).unwrap()[0].id.clone();
        repo.delete(&[id]).unwrap();
        assert!(repo.list(&m.id).unwrap().is_empty());
    }
}
