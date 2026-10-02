//! Turns capture sessions into encrypted 10 s Opus chunk files (FR-2.4, NFR-6, NFR-13).
//! Layout: `<meeting dir>/{mic,sys}/000001.opus.enc`. Each chunk is written to a `.tmp`
//! file, synced and renamed, so a crash loses at most the chunk being recorded.
//! Pause closes the capture session (the devices are released); resume continues the
//! timeline where the longer track stopped, so paused time is not stored (ADR-015).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::encoder::{ChunkEncoder, EncodedChunk, CHUNK_SAMPLES};
use super::{
    AudioBackend, AudioFrame, CaptureConfig, CaptureError, CaptureSession, Track, SAMPLE_RATE,
};
use crate::store::crypto::{chunk_aad, decrypt_chunk, encrypt_chunk, AudioKey};

const CHUNK_SUFFIX: &str = ".opus.enc";
pub const TMP_SUFFIX: &str = ".tmp";
const POLL: Duration = Duration::from_millis(50);
/// Level meters update at most 10 times a second (`recording:levels`).
pub const LEVEL_WINDOW: Duration = Duration::from_millis(100);
/// Quietest level reported, in dBFS.
pub const LEVEL_FLOOR_DB: f32 = -90.0;
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
    /// Capture ended by itself (device or audio service gone) before `stop`.
    pub capture_lost: bool,
}

/// Payload of `recording:levels`. `None` when the track delivered nothing in the window,
/// which is how a stalled device shows up (ADR-013).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Levels {
    pub mic_db: Option<f32>,
    pub sys_db: Option<f32>,
}

pub type LevelSink = Box<dyn Fn(Levels) + Send>;

enum Control {
    /// Close the session; the sender is told once the devices are released.
    Pause(Sender<()>),
    Resume(CaptureSession),
    Stop,
}

/// A running recording. `stop` (or drop) flushes the last partial chunk of each track.
pub struct Recorder {
    control: Sender<Control>,
    thread: Option<JoinHandle<Result<RecordingSummary, CaptureError>>>,
}

impl Recorder {
    pub fn start(
        backend: &dyn AudioBackend,
        config: CaptureConfig,
        target: RecordingTarget,
        levels: LevelSink,
    ) -> Result<Self, CaptureError> {
        for track in TRACKS {
            std::fs::create_dir_all(target.dir.join(track.as_str()))
                .map_err(|e| CaptureError::Storage(e.to_string()))?;
        }
        let writer = Writer::new(target, levels)?;
        let session = backend.start(config)?;
        let (control, commands) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || record(writer, session, &commands))
            .map_err(|e| CaptureError::Stream(e.to_string()))?;
        Ok(Self {
            control,
            thread: Some(thread),
        })
    }

    /// False once capture or saving failed; `stop` then reports why.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Closes the capture session and returns once the devices are released.
    pub fn pause(&self) -> Result<(), CaptureError> {
        let (ack, done) = mpsc::channel();
        self.send(Control::Pause(ack))?;
        done.recv().map_err(|_| ended())
    }

    /// Opens a new capture session and continues the recording.
    pub fn resume(
        &self,
        backend: &dyn AudioBackend,
        config: CaptureConfig,
    ) -> Result<(), CaptureError> {
        let session = backend.start(config)?;
        self.send(Control::Resume(session))
    }

    pub fn stop(mut self) -> Result<RecordingSummary, CaptureError> {
        self.join()
    }

    fn send(&self, control: Control) -> Result<(), CaptureError> {
        self.control.send(control).map_err(|_| ended())
    }

    fn join(&mut self) -> Result<RecordingSummary, CaptureError> {
        // The thread may have ended already; joining still returns its result.
        let _ = self.control.send(Control::Stop);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| CaptureError::Stream("recorder thread panicked".into()))?,
            None => Err(ended()),
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

fn ended() -> CaptureError {
    CaptureError::Stream("recorder already stopped".into())
}

/// Encoders, files and level meters for one recording. Lives on the recorder thread.
struct Writer {
    target: RecordingTarget,
    encoders: [ChunkEncoder; 2],
    /// Added to session offsets: where the current session starts on the timeline.
    base: u64,
    totals: [u64; 2],
    chunks: u32,
    meter: LevelMeter,
    levels: LevelSink,
}

impl Writer {
    fn new(target: RecordingTarget, levels: LevelSink) -> Result<Self, CaptureError> {
        Ok(Self {
            target,
            encoders: [ChunkEncoder::new(1)?, ChunkEncoder::new(2)?],
            base: 0,
            totals: [0; 2],
            chunks: 0,
            meter: LevelMeter::new(Instant::now()),
            levels,
        })
    }

    fn push(&mut self, mut frame: AudioFrame) -> Result<(), CaptureError> {
        self.meter.add(frame.track, &frame.samples);
        frame.offset_samples += self.base;
        let done = self.encoders[track_slot(frame.track)].push(&frame)?;
        self.save(frame.track, done)
    }

    fn tick(&mut self, now: Instant) {
        if let Some(levels) = self.meter.take_if_due(now) {
            (self.levels)(levels);
        }
    }

    /// Both tracks continue from the end of the longer one.
    fn rebase(&mut self) {
        self.base = self.encoders[0].position().max(self.encoders[1].position());
        self.meter = LevelMeter::new(Instant::now());
    }

    fn save(&mut self, track: Track, done: Vec<EncodedChunk>) -> Result<(), CaptureError> {
        for chunk in done {
            write_chunk(&self.target, track, &chunk)?;
            self.totals[track_slot(track)] =
                u64::from(chunk.index - 1) * CHUNK_SAMPLES + chunk.samples;
            self.chunks += 1;
        }
        Ok(())
    }

    fn finish(mut self, capture_lost: bool) -> Result<RecordingSummary, CaptureError> {
        for track in TRACKS {
            let last = self.encoders[track_slot(track)].finish()?;
            self.save(track, last.into_iter().collect())?;
        }
        let samples = self.totals[0].max(self.totals[1]);
        let summary = RecordingSummary {
            duration_ms: samples * 1_000 / u64::from(SAMPLE_RATE),
            chunks: self.chunks,
            capture_lost,
        };
        tracing::info!(
            duration_ms = summary.duration_ms,
            chunks = summary.chunks,
            capture_lost,
            "recording saved"
        );
        Ok(summary)
    }
}

/// Closes a session and saves the frames it had already queued.
fn close(writer: &mut Writer, mut session: CaptureSession) -> Result<(), CaptureError> {
    session.stop();
    while let Ok(Some(frame)) = session.recv_timeout(Duration::ZERO) {
        writer.push(frame)?;
    }
    Ok(())
}

fn record(
    mut writer: Writer,
    session: CaptureSession,
    commands: &Receiver<Control>,
) -> Result<RecordingSummary, CaptureError> {
    let mut current = Some(session);
    loop {
        let Some(session) = current.as_ref() else {
            // Paused: nothing to capture, wait for the next command.
            match commands.recv() {
                Ok(Control::Resume(session)) => {
                    writer.rebase();
                    current = Some(session);
                }
                Ok(Control::Pause(ack)) => {
                    let _ = ack.send(());
                }
                Ok(Control::Stop) | Err(_) => return writer.finish(false),
            }
            continue;
        };
        match commands.try_recv() {
            Ok(Control::Pause(ack)) => {
                if let Some(session) = current.take() {
                    close(&mut writer, session)?;
                }
                let _ = ack.send(());
                continue;
            }
            Ok(Control::Resume(extra)) => drop(extra),
            Ok(Control::Stop) | Err(TryRecvError::Disconnected) => {
                if let Some(session) = current.take() {
                    close(&mut writer, session)?;
                }
                return writer.finish(false);
            }
            Err(TryRecvError::Empty) => {}
        }
        match session.recv_timeout(POLL) {
            Ok(Some(frame)) => writer.push(frame)?,
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(%err, "capture ended before stop");
                if let Some(session) = current.take() {
                    close(&mut writer, session)?;
                }
                return writer.finish(true);
            }
        }
        writer.tick(Instant::now());
    }
}

/// RMS per track over `LEVEL_WINDOW`.
struct LevelMeter {
    window_start: Instant,
    /// Sum of squares and sample count per track.
    acc: [(f64, u64); 2],
}

impl LevelMeter {
    fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            acc: [(0.0, 0); 2],
        }
    }

    fn add(&mut self, track: Track, samples: &[f32]) {
        let acc = &mut self.acc[track_slot(track)];
        acc.0 += samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        acc.1 += samples.len() as u64;
    }

    fn take_if_due(&mut self, now: Instant) -> Option<Levels> {
        if now.duration_since(self.window_start) < LEVEL_WINDOW {
            return None;
        }
        let db = |(sum, count): (f64, u64)| {
            (count > 0).then(|| {
                let rms = (sum / count as f64).sqrt();
                (20.0 * rms.max(1e-9).log10()).clamp(f64::from(LEVEL_FLOOR_DB), 0.0) as f32
            })
        };
        let levels = Levels {
            mic_db: db(self.acc[0]),
            sys_db: db(self.acc[1]),
        };
        *self = Self::new(now);
        Some(levels)
    }
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
            script,
            ..FakeBackend::default()
        };
        let target = target(&dir.0);
        let recorder = Recorder::start(
            &backend,
            CaptureConfig::default(),
            target.clone(),
            Box::new(|_| {}),
        )
        .unwrap();
        // The fake session ends on its own once the script is delivered.
        let summary = recorder.stop().unwrap();
        assert_eq!((summary.duration_ms, summary.chunks), (12_000, 3));

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

    fn tone(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| 0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn fr_1_4_pause_releases_the_session_and_resume_keeps_tracks_aligned() {
        let dir = TempDir::new("pause");
        let backend = FakeBackend {
            // A tone, not DC: the speech encoder filters out DC.
            script: vec![(Track::Mic, tone(48_000)), (Track::Sys, tone(24_000))],
            hold_open: true,
            ..FakeBackend::default()
        };
        let target = target(&dir.0);
        let recorder = Recorder::start(
            &backend,
            CaptureConfig::default(),
            target.clone(),
            Box::new(|_| {}),
        )
        .unwrap();
        recorder.pause().unwrap();
        recorder.pause().unwrap();
        assert!(recorder.is_running());
        recorder.resume(&backend, CaptureConfig::default()).unwrap();
        let summary = recorder.stop().unwrap();
        // Session 1: mic 1 s, sys 0.5 s. Session 2 starts at 1 s on both tracks.
        assert_eq!(
            summary,
            RecordingSummary {
                duration_ms: 2_000,
                chunks: 2,
                capture_lost: false
            }
        );
        let sys = decode(&read_chunk(&target, Track::Sys, 1).unwrap());
        assert_eq!(sys.len(), 72_000);
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        assert!(rms(&sys[30_000..42_000]) < 0.01, "sys gap is not silent");
        assert!(
            rms(&sys[52_000..68_000]) > 0.05,
            "resumed sys audio missing"
        );
    }

    #[test]
    fn capture_ending_by_itself_is_reported() {
        let dir = TempDir::new("lost");
        let backend = FakeBackend {
            script: vec![(Track::Mic, vec![0.2; 4_800])],
            ..FakeBackend::default()
        };
        let recorder = Recorder::start(
            &backend,
            CaptureConfig::default(),
            target(&dir.0),
            Box::new(|_| {}),
        )
        .unwrap();
        while recorder.is_running() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let summary = recorder.stop().unwrap();
        assert!(summary.capture_lost);
        assert_eq!(summary.duration_ms, 100);
    }

    #[test]
    fn fr_1_5_level_meter_reports_db_and_none_for_silent_devices() {
        let t0 = Instant::now();
        let mut meter = LevelMeter::new(t0);
        meter.add(Track::Mic, &[0.5, -0.5, 0.5, -0.5]);
        assert_eq!(meter.take_if_due(t0 + Duration::from_millis(50)), None);
        let levels = meter.take_if_due(t0 + LEVEL_WINDOW).unwrap();
        let mic = levels.mic_db.unwrap();
        assert!((mic - -6.02).abs() < 0.01, "{mic}");
        assert_eq!(levels.sys_db, None);

        meter.add(Track::Sys, &[0.0; 10]);
        let levels = meter.take_if_due(t0 + LEVEL_WINDOW * 2).unwrap();
        assert_eq!(levels.sys_db, Some(LEVEL_FLOOR_DB));
        assert_eq!(levels.mic_db, None);
    }
}
