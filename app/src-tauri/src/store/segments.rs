//! `segments` and `speakers`: the transcript (FR-3.1, FR-3.2, FR-2.2, ADR-021).
//! Each track is written whole by its transcribe step, so a rerun replaces it.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    pub track: String,
    pub speaker_id: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

    /// Renames a speaker; every segment follows (FR-3.3). When another speaker of the same
    /// meeting and side (`is_me`) already has that name, ignoring case, this one is merged
    /// into it: its segments move over and it is deleted (ADR-022). Returns the speaker that
    /// now holds the name, `None` when `speaker_id` does not exist. `name` must be trimmed.
    pub fn rename_speaker(
        &self,
        speaker_id: &str,
        name: &str,
    ) -> Result<Option<Speaker>, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let Some((meeting_id, is_me)) = tx
            .query_row(
                "SELECT meeting_id, is_me FROM speakers WHERE id = ?1",
                [speaker_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let now = now_ms();
        let same_name: Option<String> = tx
            .query_row(
                "SELECT id FROM speakers
                 WHERE meeting_id = ?1 AND is_me = ?2 AND id != ?3 AND lower(label) = lower(?4)
                 ORDER BY rowid LIMIT 1",
                params![meeting_id, is_me, speaker_id, name],
                |row| row.get(0),
            )
            .optional()?;
        let kept = match same_name {
            Some(other) => {
                tx.execute(
                    "UPDATE segments SET speaker_id = ?2, updated_at = ?3 WHERE speaker_id = ?1",
                    params![speaker_id, other, now],
                )?;
                tx.execute("DELETE FROM speakers WHERE id = ?1", [speaker_id])?;
                other
            }
            None => speaker_id.to_owned(),
        };
        tx.execute(
            "UPDATE speakers SET label = ?2, updated_at = ?3 WHERE id = ?1",
            params![kept, name, now],
        )?;
        tx.commit()?;
        Ok(Some(Speaker {
            id: kept,
            label: name.to_owned(),
            is_me,
        }))
    }

    /// Meeting, track and times of one segment.
    pub fn locate(&self, segment_id: &str) -> Result<Option<(String, Segment)>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT meeting_id, id, track, speaker_id, start_ms, end_ms, text FROM segments
                 WHERE id = ?1",
                [segment_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        Segment {
                            id: row.get(1)?,
                            track: row.get(2)?,
                            speaker_id: row.get(3)?,
                            start_ms: row.get(4)?,
                            end_ms: row.get(5)?,
                            text: row.get(6)?,
                        },
                    ))
                },
            )
            .optional()?)
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

    #[test]
    fn fr_3_3_rename_applies_to_every_line_and_merges_a_split_voice() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let m = MeetingRepo::new(&conn)
            .create_recording("Standup", "zoom", 0)
            .unwrap();
        let repo = SegmentRepo::new(&conn);
        repo.replace_track(&m.id, Track::Mic, &[seg(0, "Hi", None)])
            .unwrap();
        repo.replace_track(
            &m.id,
            Track::Sys,
            &[
                seg(1_000, "One", Some("Speaker 1")),
                seg(2_000, "Two", Some("Speaker 2")),
                seg(3_000, "Three", Some("Speaker 1")),
            ],
        )
        .unwrap();
        let id_of = |label: &str| {
            repo.speakers(&m.id)
                .unwrap()
                .into_iter()
                .find(|s| s.label == label)
                .unwrap()
                .id
        };
        let labels = || -> Vec<String> {
            let speakers = repo.speakers(&m.id).unwrap();
            repo.list(&m.id)
                .unwrap()
                .iter()
                .map(|seg| {
                    let id = seg.speaker_id.as_ref().unwrap();
                    speakers.iter().find(|s| &s.id == id).unwrap().label.clone()
                })
                .collect()
        };
        let (me, s1, s2) = (id_of("Me"), id_of("Speaker 1"), id_of("Speaker 2"));

        struct Case {
            name: &'static str,
            speaker: String,
            to: &'static str,
            kept: String,
            lines: [&'static str; 4],
        }
        let cases = [
            Case {
                name: "plain rename",
                speaker: s1.clone(),
                to: "Ann",
                kept: s1.clone(),
                lines: ["Me", "Ann", "Speaker 2", "Ann"],
            },
            Case {
                name: "me can be renamed",
                speaker: me.clone(),
                to: "Yasin",
                kept: me.clone(),
                lines: ["Yasin", "Ann", "Speaker 2", "Ann"],
            },
            Case {
                name: "same name as me does not merge across sides",
                speaker: s2.clone(),
                to: "yasin",
                kept: s2.clone(),
                lines: ["Yasin", "Ann", "yasin", "Ann"],
            },
            Case {
                name: "same name on the same side merges, ignoring case",
                speaker: s2.clone(),
                to: "ANN",
                kept: s1.clone(),
                lines: ["Yasin", "ANN", "ANN", "ANN"],
            },
        ];
        for case in cases {
            let got = repo
                .rename_speaker(&case.speaker, case.to)
                .unwrap()
                .unwrap();
            assert_eq!(
                (got.id, got.label.as_str()),
                (case.kept, case.to),
                "{}",
                case.name
            );
            assert_eq!(labels(), case.lines, "{}", case.name);
        }
        assert_eq!(repo.speakers(&m.id).unwrap().len(), 2);
        assert_eq!(repo.rename_speaker(&s2, "Gone").unwrap(), None);

        let first = repo.list(&m.id).unwrap()[0].clone();
        assert_eq!(repo.locate(&first.id).unwrap(), Some((m.id.clone(), first)));
        assert_eq!(repo.locate("missing").unwrap(), None);
    }
}
