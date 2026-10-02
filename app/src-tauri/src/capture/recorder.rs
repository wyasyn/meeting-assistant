//! Turns a capture session into encrypted 10 s Opus chunk files (FR-2.4, NFR-6, NFR-13).
//! Layout: `<meeting dir>/{mic,sys}/000001.opus.enc`. Each chunk is written to a `.tmp`
//! file, synced and renamed, so a crash loses at most the chunk being recorded.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::encoder::{ChunkEncoder, EncodedChunk, CHUNK_SAMPLES};
use super::{AudioBackend, CaptureConfig, CaptureError, CaptureSession, Track, SAMPLE_RATE};
use crate::store::crypto::{chunk_aad, decrypt_chunk, encrypt_chunk, AudioKey};

const CHUNK_SUFFIX: &str = ".opus.enc";
pub const TMP_SUFFIX: &str = ".tmp";
const POLL: Duration = Duration::from_millis(100);
const TRACKS: [Track; 2] = [Track::Mic, Track::Sys];

pub fn chunk_file_name(index: u32) -> String {
    format!("{index:06}{CHUNK_SUFFIX}")
}

/// Index of a finished chunk file, `None` for anything else (including `.tmp`).
pub fn parse_chunk_index(file_name: &str) -> Option<u32> {
    let digits = file_name.strip_suffix(CHUNK_SUFFIX)?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|index| *index > 0)
}

/// Where a recording goes and how it is sealed.
#[derive(Debug, Clone)]
pub struct RecordingTarget {
    pub meeting_id: String,
    /// Absolute meeting folder, `<app_data>/meetings/<id>`.
    pub dir: PathBuf,
    pub key: AudioKey,
}

impl RecordingTarget {
    fn chunk_path(&self, track: Track, index: u32) -> PathBuf {
        self.dir.join(track.as_str()).join(chunk_file_name(index))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingSummary {
    /// Length of the longer track.
    pub duration_ms: u64,
    pub chunks: u32,
}

/// A running recording. `stop` (or drop) flushes the last partial chunk of each track.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<RecordingSummary, CaptureError>>>,
}

impl Recorder {
    pub fn start(
        backend: &dyn AudioBackend,
        config: CaptureConfig,
        target: RecordingTarget,
    ) -> Result<Self, CaptureError> {
        for track in TRACKS {
            std::fs::create_dir_all(target.dir.join(track.as_str()))
                .map_err(|e| CaptureError::Storage(e.to_string()))?;
        }
        let mut encoders = [ChunkEncoder::new(1)?, ChunkEncoder::new(2)?];
        let session = backend.start(config)?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || record(session, &mut encoders, &target, &flag))
            .map_err(|e| CaptureError::Stream(e.to_string()))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// False once capture or saving failed; `stop` then returns the error.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    pub fn stop(mut self) -> Result<RecordingSummary, CaptureError> {
        self.join()
    }

    fn join(&mut self) -> Result<RecordingSummary, CaptureError> {
        self.stop.store(true, Ordering::Relaxed);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| CaptureError::Stream("recorder thread panicked".into()))?,
            None => Err(CaptureError::Stream("recorder already stopped".into())),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.thread.is_some() {
            if let Err(err) = self.join() {
                tracing::warn!(%err, "recording ended with an error");
            }
        }
    }
}

fn record(
    mut session: CaptureSession,
    encoders: &mut [ChunkEncoder; 2],
    target: &RecordingTarget,
    stop: &AtomicBool,
) -> Result<RecordingSummary, CaptureError> {
    let mut totals = [0u64; 2];
    let mut chunks = 0u32;
    let mut handle = |track: Track, done: Vec<EncodedChunk>| -> Result<(), CaptureError> {
        for chunk in done {
            write_chunk(target, track, &chunk)?;
            let slot = track_slot(track);
            totals[slot] = u64::from(chunk.index - 1) * CHUNK_SAMPLES + chunk.samples;
            chunks += 1;
        }
        Ok(())
    };

    while !stop.load(Ordering::Relaxed) {
        match session.recv_timeout(POLL) {
            Ok(Some(frame)) => {
                let done = encoders[track_slot(frame.track)].push(&frame)?;
                handle(frame.track, done)?;
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(%err, "capture ended before stop");
                break;
            }
        }
    }
    session.stop();
    // Frames already queued when capture stopped.
    while let Ok(Some(frame)) = session.recv_timeout(Duration::ZERO) {
        let done = encoders[track_slot(frame.track)].push(&frame)?;
        handle(frame.track, done)?;
    }
    for track in TRACKS {
        let last = encoders[track_slot(track)].finish()?;
        handle(track, last.into_iter().collect())?;
    }

    let samples = totals[0].max(totals[1]);
    let summary = RecordingSummary {
        duration_ms: samples * 1_000 / u64::from(SAMPLE_RATE),
        chunks,
    };
    tracing::info!(
        duration_ms = summary.duration_ms,
        chunks = summary.chunks,
        "recording saved"
    );
    Ok(summary)
}

fn track_slot(track: Track) -> usize {
    match track {
        Track::Mic => 0,
        Track::Sys => 1,
    }
}

fn write_chunk(
    target: &RecordingTarget,
    track: Track,
    chunk: &EncodedChunk,
) -> Result<(), CaptureError> {
    let aad = chunk_aad(&target.meeting_id, track.as_str(), chunk.index);
    let sealed = encrypt_chunk(&target.key, &aad, &chunk.ogg)
        .map_err(|e| CaptureError::Storage(e.to_string()))?;
    let path = target.chunk_path(track, chunk.index);
    write_atomic(&path, &sealed).map_err(|e| CaptureError::Storage(e.to_string()))?;
    tracing::debug!(path = %path.display(), "chunk saved");
    Ok(())
}

/// Write to `<name>.tmp`, sync, rename over `<name>`, sync the folder.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp_name);
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// Reads and decrypts one chunk back to Ogg Opus bytes (for the pipeline, roadmap 1.9).
pub fn read_chunk(
    target: &RecordingTarget,
    track: Track,
    index: u32,
) -> Result<Vec<u8>, CaptureError> {
    let sealed = std::fs::read(target.chunk_path(track, index))
        .map_err(|e| CaptureError::Storage(e.to_string()))?;
    let aad = chunk_aad(&target.meeting_id, track.as_str(), index);
    decrypt_chunk(&target.key, &aad, &sealed).map_err(|e| CaptureError::Storage(e.to_string()))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::capture::encoder::tests::decode;
    use crate::capture::fake::FakeBackend;
    use crate::store::key::KEY_LEN;

    /// A folder under the system temp dir, removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ma-capture-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn target(dir: &Path) -> RecordingTarget {
        RecordingTarget {
            meeting_id: "m1".into(),
            dir: dir.to_path_buf(),
            key: AudioKey::new(&[3; KEY_LEN]).unwrap(),
        }
    }

    #[test]
    fn chunk_names_round_trip_and_reject_others() {
        assert_eq!(chunk_file_name(1), "000001.opus.enc");
        assert_eq!(parse_chunk_index("000042.opus.enc"), Some(42));
        assert_eq!(parse_chunk_index("000042.opus.enc.tmp"), None);
        assert_eq!(parse_chunk_index("000000.opus.enc"), None);
        assert_eq!(parse_chunk_index("42.opus.enc"), None);
        assert_eq!(parse_chunk_index("notes.txt"), None);
    }

    #[test]
    fn fr_2_4_records_both_tracks_into_encrypted_chunks() {
        let dir = TempDir::new("record");
        // 12 s of mic and 3 s of system audio, in 0.5 s frames.
        let mut script = Vec::new();
        for _ in 0..24 {
            script.push((Track::Mic, vec![0.2; 24_000]));
        }
        for _ in 0..6 {
            script.push((Track::Sys, vec![0.1; 24_000]));
        }
        let backend = FakeBackend {
            devices: vec![],
            script,
        };
        let target = target(&dir.0);
        let recorder = Recorder::start(&backend, CaptureConfig::default(), target.clone()).unwrap();
        // The fake session ends on its own once the script is delivered.
        let summary = recorder.stop().unwrap();
        assert_eq!(
            summary,
            RecordingSummary {
                duration_ms: 12_000,
                chunks: 3
            }
        );

        let files = |track: &str| {
            let mut names: Vec<String> = std::fs::read_dir(dir.0.join(track))
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        assert_eq!(files("mic"), vec!["000001.opus.enc", "000002.opus.enc"]);
        assert_eq!(files("sys"), vec!["000001.opus.enc"]);

        let raw = std::fs::read(dir.0.join("mic").join("000001.opus.enc")).unwrap();
        assert!(
            !raw.windows(4).any(|w| w == b"OggS"),
            "chunk is not encrypted"
        );
        assert_eq!(
            decode(&read_chunk(&target, Track::Mic, 2).unwrap()).len(),
            2 * 48_000
        );
        assert_eq!(
            decode(&read_chunk(&target, Track::Sys, 1).unwrap()).len(),
            3 * 48_000
        );
    }
}
