//! Startup scan for recordings cut off by a crash or power loss (NFR-6, rule 3).
//! Each meeting still marked `recording` becomes `interrupted`; its saved chunks are kept,
//! half-written `.tmp` files are removed, and its length comes from the saved audio.

use std::path::Path;

use super::encoder::{chunk_samples, CHUNK_SAMPLES};
use super::recorder::{parse_chunk_index, read_chunk, RecordingTarget, TMP_SUFFIX};
use super::{Track, SAMPLE_RATE};
use crate::store::meetings::{MeetingRepo, MeetingStatus, OpenRecording};
use crate::store::{Store, StoreError};

/// Marks interrupted recordings and returns how many there were.
pub fn recover(store: &Store, data_dir: &Path) -> Result<usize, StoreError> {
    let open = MeetingRepo::new(&*store.conn()?).open_recordings()?;
    for meeting in &open {
        let target = RecordingTarget {
            meeting_id: meeting.id.clone(),
            dir: data_dir.join(&meeting.audio_dir),
            key: store.audio_key().clone(),
        };
        let duration_ms = [Track::Mic, Track::Sys]
            .into_iter()
            .map(|track| saved_ms(&target, track))
            .max()
            .unwrap_or(0);
        finish(store, meeting, duration_ms)?;
    }
    if !open.is_empty() {
        tracing::info!(count = open.len(), "interrupted recordings recovered");
    }
    Ok(open.len())
}

fn finish(store: &Store, meeting: &OpenRecording, duration_ms: u64) -> Result<(), StoreError> {
    let duration_ms = i64::try_from(duration_ms).unwrap_or(i64::MAX);
    MeetingRepo::new(&*store.conn()?).finish(
        &meeting.id,
        meeting.started_at.saturating_add(duration_ms),
        (duration_ms + 500) / 1_000,
        MeetingStatus::Interrupted,
    )
}

/// Length of the saved audio of one track. Removes `.tmp` leftovers on the way.
fn saved_ms(target: &RecordingTarget, track: Track) -> u64 {
    let dir = target.dir.join(track.as_str());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut last = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.ends_with(TMP_SUFFIX) {
            if let Err(err) = std::fs::remove_file(entry.path()) {
                tracing::warn!(%err, "could not remove a partial chunk");
            }
        } else if let Some(index) = parse_chunk_index(name) {
            last = last.max(index);
        }
    }
    if last == 0 {
        return 0;
    }
    // A damaged last chunk counts as full; the pipeline reports it when it reads it.
    let last_samples = read_chunk(target, track, last)
        .ok()
        .and_then(|ogg| chunk_samples(&ogg))
        .unwrap_or(CHUNK_SAMPLES);
    let samples = u64::from(last - 1) * CHUNK_SAMPLES + last_samples;
    samples * 1_000 / u64::from(SAMPLE_RATE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::encoder::ChunkEncoder;
    use crate::capture::recorder::chunk_file_name;
    use crate::capture::recorder::tests::TempDir;
    use crate::capture::AudioFrame;
    use crate::store::crypto::{chunk_aad, encrypt_chunk};

    /// Writes `seconds` of audio for `track` as chunk files, like the recorder.
    fn save(target: &RecordingTarget, track: Track, seconds: u64) {
        let dir = target.dir.join(track.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        let mut enc = ChunkEncoder::new(1).unwrap();
        let mut chunks = enc
            .push(&AudioFrame {
                track,
                offset_samples: 0,
                samples: vec![0.1; (seconds * 48_000) as usize],
            })
            .unwrap();
        chunks.extend(enc.finish().unwrap());
        for chunk in chunks {
            let aad = chunk_aad(&target.meeting_id, track.as_str(), chunk.index);
            let sealed = encrypt_chunk(&target.key, &aad, &chunk.ogg).unwrap();
            std::fs::write(dir.join(chunk_file_name(chunk.index)), sealed).unwrap();
        }
    }

    #[test]
    fn nfr_6_interrupted_recording_keeps_audio_and_gets_its_length() {
        let data = TempDir::new("recover");
        let store = Store::open_in_memory().unwrap();
        let (cut, done) = {
            let conn = store.conn().unwrap();
            let repo = MeetingRepo::new(&conn);
            let cut = repo.create_recording("Cut", "zoom", 1_000).unwrap();
            let done = repo.create_recording("Done", "meet", 2_000).unwrap();
            repo.finish(&done.id, 3_000, 1, MeetingStatus::Ready)
                .unwrap();
            (cut, done)
        };
        let target = RecordingTarget {
            meeting_id: cut.id.clone(),
            dir: data.0.join(&cut.audio_dir),
            key: store.audio_key().clone(),
        };
        save(&target, Track::Mic, 23);
        save(&target, Track::Sys, 4);
        let tmp = target.dir.join("mic").join("000004.opus.enc.tmp");
        std::fs::write(&tmp, b"half written").unwrap();

        assert_eq!(recover(&store, &data.0).unwrap(), 1);

        let conn = store.conn().unwrap();
        let repo = MeetingRepo::new(&conn);
        assert_eq!(
            repo.status(&cut.id).unwrap().as_deref(),
            Some("interrupted")
        );
        assert_eq!(repo.status(&done.id).unwrap().as_deref(), Some("ready"));
        let (ended_at, duration_s): (i64, i64) = conn
            .query_row(
                "SELECT ended_at, duration_s FROM meetings WHERE id = ?1",
                [&cut.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((ended_at, duration_s), (1_000 + 23_000, 23));
        assert!(!tmp.exists());
        assert!(target.dir.join("mic").join("000003.opus.enc").exists());
        assert!(repo.open_recordings().unwrap().is_empty());
    }

    #[test]
    fn nfr_6_recording_with_no_saved_chunks_is_still_closed() {
        let data = TempDir::new("recover-empty");
        let store = Store::open_in_memory().unwrap();
        let id = MeetingRepo::new(&store.conn().unwrap())
            .create_recording("Short", "zoom", 5_000)
            .unwrap()
            .id;
        assert_eq!(recover(&store, &data.0).unwrap(), 1);
        let conn = store.conn().unwrap();
        assert_eq!(
            MeetingRepo::new(&conn).status(&id).unwrap().as_deref(),
            Some("interrupted")
        );
        assert_eq!(recover(&store, &data.0).unwrap(), 0);
    }
}
