//! Reading meetings back: recent meetings, one meeting's transcript, renaming speakers
//! and playing a line (FR-3.3, FR-3.8, FR-6.1 partial, ADR-022).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::capture::playback::track_wav;
use crate::capture::recorder::RecordingTarget;
use crate::capture::Track;
use crate::error::AppError;
use crate::recording::MeetingSummary;
use crate::store::highlights::{Highlight, HighlightRepo};
use crate::store::meetings::{MeetingRepo, MeetingRow};
use crate::store::reports::{ActionItem, Report, ReportRepo, Score, ScoreValue};
use crate::store::segments::{Segment, SegmentRepo, Speaker};
use crate::store::Store;

pub const DEFAULT_PAGE: u32 = 50;
pub const MAX_PAGE: u32 = 200;
const MAX_NAME_CHARS: usize = 100;
const NO_MEETING: &str = "This meeting no longer exists.";
const DAMAGED: &str = "A recording file could not be read. It may be damaged.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass back as `cursor` for the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

/// `export_meeting` formats (docs/06). Only Markdown is written by the core; PDF comes
/// from the window's print dialog and the others are not built yet (ADR-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Md,
    Pdf,
    Docx,
    Srt,
    Json,
}

/// One row of the library (FR-6.1): the meeting, who was in it and its scores.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingListItem {
    #[serde(flatten)]
    pub meeting: MeetingSummary,
    /// Names of the other side, in order of first appearance; "Me" is left out.
    pub participants: Vec<String>,
    /// Headline scores in display order; empty until scored or with too little data.
    pub scores: Vec<ScoreValue>,
}

/// `get_meeting`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub meeting: MeetingSummary,
    /// "Me" first, then the other side in order of first appearance.
    pub speakers: Vec<Speaker>,
    /// Both tracks, in time order.
    pub segments: Vec<Segment>,
    /// False once retention removed the audio; lines can no longer be played.
    pub has_audio: bool,
    /// `None` until analyzed, or when nobody was heard.
    pub report: Option<Report>,
    pub action_items: Vec<ActionItem>,
    /// Headline scores; empty when there was too little data to score.
    pub scores: Vec<Score>,
    /// Moments the user marked while recording, in time order (FR-9.1).
    pub highlights: Vec<Highlight>,
}

#[derive(Clone)]
pub struct MeetingService {
    store: Arc<Store>,
    /// `<app_data>`; meetings store their audio dir relative to it.
    data_dir: PathBuf,
}

impl MeetingService {
    pub fn new(store: Arc<Store>, data_dir: PathBuf) -> Self {
        Self { store, data_dir }
    }

    /// Newest first. `cursor` comes from the previous page's `next_cursor`.
    pub fn list(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<MeetingListItem>, AppError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let before = cursor.map(parse_cursor).transpose()?;
        let conn = self.store.conn()?;
        let rows = MeetingRepo::new(&conn)
            .list(before.as_ref().map(|(at, id)| (*at, id.as_str())), limit)?;
        let next_cursor = (rows.len() == limit as usize)
            .then(|| rows.last().map(|r| format!("{}:{}", r.started_at, r.id)))
            .flatten();
        let (segments, reports) = (SegmentRepo::new(&conn), ReportRepo::new(&conn));
        let items = rows
            .into_iter()
            .map(|row| {
                let participants = segments
                    .speakers(&row.id)?
                    .into_iter()
                    .filter(|s| !s.is_me)
                    .map(|s| s.label)
                    .collect();
                Ok(MeetingListItem {
                    participants,
                    scores: reports.headline_values(&row.id)?,
                    meeting: summary(row),
                })
            })
            .collect::<Result<_, AppError>>()?;
        Ok(Page { items, next_cursor })
    }

    pub fn detail(&self, id: &str) -> Result<MeetingDetail, AppError> {
        let conn = self.store.conn()?;
        let row = MeetingRepo::new(&conn)
            .get(id)?
            .ok_or_else(|| AppError::NotFound(NO_MEETING.into()))?;
        let segments = SegmentRepo::new(&conn);
        let reports = ReportRepo::new(&conn);
        Ok(MeetingDetail {
            speakers: segments.speakers(id)?,
            segments: segments.list(id)?,
            has_audio: row.audio_deleted_at.is_none(),
            meeting: summary(row),
            report: reports.get(id)?,
            action_items: reports.action_items(id)?,
            scores: reports.headline_scores(id)?,
            highlights: HighlightRepo::new(&conn).list(id)?,
        })
    }

    /// Every line of the speaker follows; a name another speaker on the same side already
    /// has merges the two (ADR-022). Returns the speaker that now holds the name.
    pub fn rename_speaker(&self, speaker_id: &str, name: &str) -> Result<Speaker, AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::InvalidState("Enter a name.".into()));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(AppError::InvalidState(format!(
                "Use a name of {MAX_NAME_CHARS} characters or fewer."
            )));
        }
        SegmentRepo::new(&*self.store.conn()?)
            .rename_speaker(speaker_id, name)?
            .ok_or_else(|| AppError::NotFound("This speaker no longer exists.".into()))
    }

    /// The segment's stretch of its own track as a WAV file, in memory (FR-3.8).
    pub fn segment_audio(&self, segment_id: &str) -> Result<Vec<u8>, AppError> {
        let (target, segment) = {
            let conn = self.store.conn()?;
            let (meeting_id, segment) = SegmentRepo::new(&conn)
                .locate(segment_id)?
                .ok_or_else(|| AppError::NotFound("This line no longer exists.".into()))?;
            let row = MeetingRepo::new(&conn)
                .get(&meeting_id)?
                .ok_or_else(|| AppError::NotFound(NO_MEETING.into()))?;
            if row.audio_deleted_at.is_some() {
                return Err(AppError::NotFound(
                    "The audio of this meeting was deleted.".into(),
                ));
            }
            let target = RecordingTarget {
                meeting_id,
                dir: self.data_dir.join(&row.audio_dir),
                key: self.store.audio_key().clone(),
            };
            (target, segment)
        };
        let track = match segment.track.as_str() {
            "mic" => Track::Mic,
            _ => Track::Sys,
        };
        track_wav(&target, track, segment.start_ms, segment.end_ms).map_err(|e| {
            tracing::debug!(error = %e, "could not decode a segment");
            AppError::Storage(DAMAGED.into())
        })
    }
}

impl MeetingService {
    /// Writes an export the window rendered to the file the user picked in the save dialog
    /// (FR-7.1). The file appears whole or not at all. Returns the path written.
    pub fn export(
        &self,
        id: &str,
        format: ExportFormat,
        path: &Path,
        text: &str,
    ) -> Result<String, AppError> {
        if format != ExportFormat::Md {
            return Err(AppError::InvalidState(
                "Only Markdown files can be saved for now. Use Export PDF to print to a PDF."
                    .into(),
            ));
        }
        if MeetingRepo::new(&*self.store.conn()?).get(id)?.is_none() {
            return Err(AppError::NotFound(NO_MEETING.into()));
        }
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(AppError::InvalidState("Choose a file to save to.".into()));
        };
        if !path.is_absolute() {
            return Err(AppError::InvalidState("Choose a file to save to.".into()));
        }
        let tmp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
        let written = std::fs::File::create(&tmp)
            .and_then(|mut f| {
                f.write_all(text.as_bytes())?;
                f.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!(error = %e, "export failed");
            return Err(AppError::Storage(
                "Could not save the file. Check that you can write to that folder.".into(),
            ));
        }
        tracing::info!("meeting exported");
        Ok(path.to_string_lossy().into_owned())
    }
}

fn summary(row: MeetingRow) -> MeetingSummary {
    MeetingSummary {
        id: row.id,
        title: row.title,
        source_app: row.source_app,
        started_at: row.started_at,
        ended_at: row.ended_at,
        duration_s: row.duration_s,
        status: row.status,
    }
}

fn parse_cursor(cursor: &str) -> Result<(i64, String), AppError> {
    cursor
        .split_once(':')
        .and_then(|(at, id)| Some((at.parse().ok()?, id.to_owned())))
        .ok_or_else(|| AppError::InvalidState("This list is out of date. Reload it.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::recorder::tests::TempDir;
    use crate::store::meetings::MeetingStatus;
    use crate::store::segments::NewSegment;

    struct Harness {
        _dir: TempDir,
        service: MeetingService,
        store: Arc<Store>,
        meeting_id: String,
    }

    fn harness(name: &str) -> Harness {
        let dir = TempDir::new(&format!("meetings-{name}"));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let meeting_id = {
            let conn = store.conn().unwrap();
            let repo = MeetingRepo::new(&conn);
            let m = repo.create_recording("Standup", "zoom", 1_000).unwrap();
            repo.finish(&m.id, 31_000, 30, MeetingStatus::Ready)
                .unwrap();
            let seg = |start_ms, text: &str, label: Option<&str>| NewSegment {
                start_ms,
                end_ms: start_ms + 1_000,
                text: text.into(),
                speaker_label: label.map(Into::into),
            };
            let segments = SegmentRepo::new(&conn);
            segments
                .replace_track(&m.id, Track::Mic, &[seg(2_000, "Hi", None)])
                .unwrap();
            segments
                .replace_track(&m.id, Track::Sys, &[seg(1_000, "Hello", Some("Speaker 1"))])
                .unwrap();
            m.id
        };
        Harness {
            service: MeetingService::new(Arc::clone(&store), dir.0.clone()),
            _dir: dir,
            store,
            meeting_id,
        }
    }

    #[test]
    fn fr_6_1_pages_follow_the_cursor() {
        let h = harness("list");
        {
            let conn = h.store.conn().unwrap();
            let repo = MeetingRepo::new(&conn);
            repo.create_recording("Later", "browser", 5_000).unwrap();
        }
        let first = h.service.list(None, Some(1)).unwrap();
        assert_eq!(first.items[0].meeting.title, "Later");
        assert!(first.items[0].participants.is_empty());
        let cursor = first.next_cursor.unwrap();
        let second = h.service.list(Some(&cursor), Some(1)).unwrap();
        assert_eq!(second.items[0].meeting.title, "Standup");
        assert_eq!(second.items[0].meeting.duration_s, Some(30));
        assert_eq!(second.items[0].participants, ["Speaker 1"]);
        assert!(second.items[0].scores.is_empty());
        let wire = serde_json::to_value(&second.items[0]).unwrap();
        assert_eq!(
            (&wire["title"], &wire["participants"][0]),
            (
                &serde_json::json!("Standup"),
                &serde_json::json!("Speaker 1")
            )
        );
        let last = h.service.list(Some(&cursor), None).unwrap();
        assert_eq!((last.items.len(), last.next_cursor), (1, None));
        let bad = h.service.list(Some("nonsense"), None).unwrap_err();
        assert_eq!(bad.code(), "invalid_state");
    }

    #[test]
    fn fr_3_3_detail_and_rename() {
        let h = harness("detail");
        let detail = h.service.detail(&h.meeting_id).unwrap();
        assert_eq!(detail.meeting.status, "ready");
        assert!(detail.has_audio);
        let texts: Vec<_> = detail.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "Hi"]);
        let labels: Vec<_> = detail
            .speakers
            .iter()
            .map(|s| (s.label.as_str(), s.is_me))
            .collect();
        assert_eq!(labels, [("Me", true), ("Speaker 1", false)]);
        assert_eq!(detail.report, None);
        assert!(detail.action_items.is_empty() && detail.scores.is_empty());
        assert!(detail.highlights.is_empty());

        let other = &detail.speakers[1];
        let renamed = h.service.rename_speaker(&other.id, "  Ann ").unwrap();
        assert_eq!(renamed.label, "Ann");
        assert_eq!(
            h.service.rename_speaker(&other.id, " ").unwrap_err().code(),
            "invalid_state"
        );
        assert_eq!(
            h.service
                .rename_speaker(&other.id, &"x".repeat(101))
                .unwrap_err()
                .code(),
            "invalid_state"
        );
        assert_eq!(
            h.service
                .rename_speaker("missing", "Ann")
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(h.service.detail("missing").unwrap_err().code(), "not_found");
    }

    #[test]
    fn fr_7_1_markdown_export_is_written_whole() {
        let h = harness("export");
        let path = h.service.data_dir.join("Standup.md");
        let written = h
            .service
            .export(&h.meeting_id, ExportFormat::Md, &path, "# Standup\n")
            .unwrap();
        assert_eq!(written, path.to_string_lossy());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Standup\n");
        // Saving again replaces the file and leaves no temporary file behind.
        h.service
            .export(&h.meeting_id, ExportFormat::Md, &path, "# Again\n")
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Again\n");
        assert_eq!(std::fs::read_dir(&h.service.data_dir).unwrap().count(), 1);

        let err = |id: &str, format, path: &Path| {
            h.service.export(id, format, path, "x").unwrap_err().code()
        };
        assert_eq!(
            err(&h.meeting_id, ExportFormat::Pdf, &path),
            "invalid_state"
        );
        assert_eq!(err("missing", ExportFormat::Md, &path), "not_found");
        assert_eq!(
            err(&h.meeting_id, ExportFormat::Md, Path::new("relative.md")),
            "invalid_state"
        );
        let nowhere = h.service.data_dir.join("no-such-dir").join("x.md");
        assert_eq!(err(&h.meeting_id, ExportFormat::Md, &nowhere), "storage");
        assert_eq!(
            serde_json::from_value::<ExportFormat>("md".into()).unwrap(),
            ExportFormat::Md
        );
    }

    #[test]
    fn fr_3_8_segment_audio_reads_its_own_track() {
        let h = harness("audio");
        let detail = h.service.detail(&h.meeting_id).unwrap();
        // No chunks on disk: the line plays as silence of its length.
        let wav = h.service.segment_audio(&detail.segments[0].id).unwrap();
        assert_eq!(wav.len(), 44 + 1_000 * 16 * 2);
        assert_eq!(
            h.service.segment_audio("missing").unwrap_err().code(),
            "not_found"
        );

        // A damaged chunk is a readable storage error.
        let dir = h
            .service
            .data_dir
            .join("meetings")
            .join(&h.meeting_id)
            .join("sys");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("000001.opus.enc"), b"garbage").unwrap();
        let err = h.service.segment_audio(&detail.segments[0].id).unwrap_err();
        assert_eq!((err.code(), err.to_string().as_str()), ("storage", DAMAGED));

        h.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE meetings SET audio_deleted_at = 1 WHERE id = ?1",
                [&h.meeting_id],
            )
            .unwrap();
        assert!(!h.service.detail(&h.meeting_id).unwrap().has_audio);
        assert_eq!(
            h.service
                .segment_audio(&detail.segments[0].id)
                .unwrap_err()
                .code(),
            "not_found"
        );
    }
}
