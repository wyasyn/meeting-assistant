//! Manual recording: start, pause, resume, stop (FR-1.4, FR-1.5, NFR-12, ADR-015).
//! Starts only from a user action (rule 1). Every change is announced through
//! `RecordingEvents`, which drives the `recording:state` event and the tray indicator.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::capture::recorder::{Levels, Recorder, RecordingTarget};
use crate::capture::{AudioBackend, CaptureConfig, CaptureError};
use crate::detector::apps::SourceApp;
use crate::detector::ActiveRecording;
use crate::error::AppError;
use crate::store::highlights::{Highlight, HighlightRepo};
use crate::store::meetings::{MeetingRepo, MeetingStatus, OpenRecording};
use crate::store::{now_ms, Store};

const WATCHDOG_INTERVAL: Duration = Duration::from_millis(500);
/// Both tracks this quiet for `SILENCE_AFTER` suggests the meeting is over (FR-1.6).
pub const SILENCE_DB: f32 = -50.0;
pub const SILENCE_AFTER: Duration = Duration::from_secs(120);
/// A longer gap between level updates is a pause; quiet time restarts after it.
const LEVEL_GAP: Duration = Duration::from_secs(1);
const DEFAULT_TITLE: &str = "Recording";
const DEFAULT_SOURCE_APP: &str = "other";
const MAX_NOTE_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// No recording since launch.
    Idle,
    Recording,
    Paused,
    Stopped,
}

/// Payload of `recording:state` and the return value of the recording commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
    pub meeting_id: Option<String>,
    pub state: Phase,
    pub elapsed_ms: u64,
    /// Why a recording stopped by itself. Readable, never a path.
    pub error: Option<String>,
}

impl RecordingState {
    fn idle() -> Self {
        Self {
            meeting_id: None,
            state: Phase::Idle,
            elapsed_ms: 0,
            error: None,
        }
    }
}

/// Returned by `start_recording`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    pub source_app: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_s: Option<i64>,
    pub status: String,
}

/// Where state changes and levels go: Tauri events and the tray in the app, a fake in tests.
pub trait RecordingEvents: Send + Sync {
    fn state(&self, state: &RecordingState);
    fn levels(&self, levels: Levels);
    /// Both tracks were quiet for `SILENCE_AFTER` (FR-1.6). Called on the recorder thread.
    fn silent(&self, meeting_id: &str);
    /// The user marked a moment (FR-9.1).
    fn highlight(&self, _highlight: &Highlight) {}
}

/// Spots `SILENCE_AFTER` of quiet on both tracks; reports once per quiet stretch.
#[derive(Debug, Default)]
struct SilenceTracker {
    last_update: Option<Instant>,
    quiet_since: Option<Instant>,
    reported: bool,
}

impl SilenceTracker {
    /// True when this update completes a quiet stretch.
    fn update(&mut self, levels: Levels, now: Instant) -> bool {
        let resumed = self
            .last_update
            .is_none_or(|last| now.saturating_duration_since(last) > LEVEL_GAP);
        self.last_update = Some(now);
        // No audio from a device counts as quiet.
        let quiet = |db: Option<f32>| db.is_none_or(|db| db < SILENCE_DB);
        if !(quiet(levels.mic_db) && quiet(levels.sys_db)) {
            self.quiet_since = None;
            self.reported = false;
            return false;
        }
        if resumed {
            self.quiet_since = None;
        }
        let since = *self.quiet_since.get_or_insert(now);
        if self.reported || now.saturating_duration_since(since) < SILENCE_AFTER {
            return false;
        }
        self.reported = true;
        true
    }
}

struct Active {
    recorder: Recorder,
    meeting: OpenRecording,
    source_app: Option<SourceApp>,
    /// Recorded time before the current stretch.
    elapsed: Duration,
    /// Set while recording, `None` while paused.
    resumed_at: Option<Instant>,
}

impl Active {
    fn elapsed(&self) -> Duration {
        self.elapsed + self.resumed_at.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn state(&self) -> RecordingState {
        RecordingState {
            meeting_id: Some(self.meeting.id.clone()),
            state: if self.resumed_at.is_some() {
                Phase::Recording
            } else {
                Phase::Paused
            },
            elapsed_ms: duration_ms(self.elapsed()),
            error: None,
        }
    }
}

struct Inner {
    active: Option<Active>,
    last: RecordingState,
}

struct Shared {
    backend: Box<dyn AudioBackend>,
    store: Arc<Store>,
    data_dir: PathBuf,
    events: Arc<dyn RecordingEvents>,
    inner: Mutex<Inner>,
}

/// Managed as Tauri state. Cheap to clone.
#[derive(Clone)]
pub struct RecordingService(Arc<Shared>);

impl RecordingService {
    pub fn new(
        backend: Box<dyn AudioBackend>,
        store: Arc<Store>,
        data_dir: PathBuf,
        events: Arc<dyn RecordingEvents>,
    ) -> Self {
        Self(Arc::new(Shared {
            backend,
            store,
            data_dir,
            events,
            inner: Mutex::new(Inner {
                active: None,
                last: RecordingState::idle(),
            }),
        }))
    }

    pub fn state(&self) -> Result<RecordingState, AppError> {
        let inner = self.lock()?;
        Ok(inner
            .active
            .as_ref()
            .map_or_else(|| inner.last.clone(), Active::state))
    }

    /// The recording in progress, running or paused.
    pub fn active(&self) -> Option<ActiveRecording> {
        let inner = self.lock().ok()?;
        inner.active.as_ref().map(|active| ActiveRecording {
            meeting_id: active.meeting.id.clone(),
            source_app: active.source_app,
        })
    }

    /// Marks the current moment of the recording, running or paused (FR-9.1). From the
    /// window, the tray or `--highlight` on the command line (ADR-029).
    pub fn add_highlight(&self, note: Option<&str>) -> Result<Highlight, AppError> {
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        if note.is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return Err(AppError::InvalidState(format!(
                "Use a note of {MAX_NOTE_CHARS} characters or fewer."
            )));
        }
        let (meeting_id, at_ms) = {
            let inner = self.lock()?;
            let active = inner.active.as_ref().ok_or_else(|| {
                AppError::InvalidState("Highlights can be added while recording.".into())
            })?;
            let at_ms = i64::try_from(duration_ms(active.elapsed())).unwrap_or(i64::MAX);
            (active.meeting.id.clone(), at_ms)
        };
        let highlight = HighlightRepo::new(&*self.0.store.conn()?).add(&meeting_id, at_ms, note)?;
        tracing::info!("highlight added");
        self.0.events.highlight(&highlight);
        Ok(highlight)
    }

    pub fn start(
        &self,
        title: Option<String>,
        source_app: Option<String>,
    ) -> Result<MeetingSummary, AppError> {
        let mut inner = self.lock()?;
        if inner.active.is_some() {
            return Err(AppError::InvalidState(
                "A recording is already running.".into(),
            ));
        }
        let source_app = source_app.unwrap_or_else(|| DEFAULT_SOURCE_APP.into());
        let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
            SourceApp::parse(&source_app)
                .map_or_else(|| DEFAULT_TITLE.into(), SourceApp::meeting_title)
        });
        let started_at = now_ms();
        let meeting = MeetingRepo::new(&*self.0.store.conn()?).create_recording(
            &title,
            &source_app,
            started_at,
        )?;
        let target = RecordingTarget {
            meeting_id: meeting.id.clone(),
            dir: self.0.data_dir.join(&meeting.audio_dir),
            key: self.0.store.audio_key().clone(),
        };
        let events = Arc::clone(&self.0.events);
        let silence = Mutex::new(SilenceTracker::default());
        let meeting_id = meeting.id.clone();
        let started = Recorder::start(
            &*self.0.backend,
            CaptureConfig::default(),
            target.clone(),
            Box::new(move |levels| {
                events.levels(levels);
                let silent = silence
                    .lock()
                    .is_ok_and(|mut s| s.update(levels, Instant::now()));
                if silent {
                    events.silent(&meeting_id);
                }
            }),
        );
        let recorder = match started {
            Ok(recorder) => recorder,
            Err(err) => {
                self.discard(&meeting.id, &target.dir);
                return Err(err.into());
            }
        };
        let active = Active {
            recorder,
            meeting: meeting.clone(),
            source_app: SourceApp::parse(&source_app),
            elapsed: Duration::ZERO,
            resumed_at: Some(Instant::now()),
        };
        let state = active.state();
        inner.active = Some(active);
        drop(inner);
        tracing::info!("recording started");
        self.0.events.state(&state);
        self.spawn_watchdog(meeting.id.clone());
        Ok(MeetingSummary {
            id: meeting.id,
            title,
            source_app,
            started_at,
            ended_at: None,
            duration_s: None,
            status: MeetingStatus::Recording.as_str().into(),
        })
    }

    pub fn pause(&self) -> Result<RecordingState, AppError> {
        let mut inner = self.lock()?;
        let active = inner
            .active
            .as_mut()
            .filter(|a| a.resumed_at.is_some())
            .ok_or_else(|| AppError::InvalidState("Nothing is being recorded.".into()))?;
        active.recorder.pause()?;
        active.elapsed = active.elapsed();
        active.resumed_at = None;
        let state = active.state();
        drop(inner);
        tracing::info!("recording paused");
        self.0.events.state(&state);
        Ok(state)
    }

    pub fn resume(&self) -> Result<RecordingState, AppError> {
        let mut inner = self.lock()?;
        let active = inner
            .active
            .as_mut()
            .filter(|a| a.resumed_at.is_none())
            .ok_or_else(|| AppError::InvalidState("The recording is not paused.".into()))?;
        active
            .recorder
            .resume(&*self.0.backend, CaptureConfig::default())?;
        active.resumed_at = Some(Instant::now());
        let state = active.state();
        drop(inner);
        tracing::info!("recording resumed");
        self.0.events.state(&state);
        Ok(state)
    }

    /// Saves the last chunks and hands the meeting to processing. A recording that fails
    /// to save its tail still ends; the state then carries the error.
    pub fn stop(&self) -> Result<RecordingState, AppError> {
        let mut inner = self.lock()?;
        let active = inner
            .active
            .take()
            .ok_or_else(|| AppError::InvalidState("Nothing is being recorded.".into()))?;
        let state = self.0.finish(active, false);
        inner.last = state.clone();
        drop(inner);
        self.0.events.state(&state);
        Ok(state)
    }

    /// Stops and saves any recording; for app exit.
    pub fn shutdown(&self) {
        if let Err(err) = self.stop() {
            if !matches!(err, AppError::InvalidState(_)) {
                tracing::warn!(%err, "could not stop the recording on exit");
            }
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, AppError> {
        self.0.lock()
    }

    fn discard(&self, meeting_id: &str, dir: &std::path::Path) {
        let deleted = self
            .0
            .store
            .conn()
            .map_err(AppError::from)
            .and_then(|conn| Ok(MeetingRepo::new(&conn).delete(meeting_id)?));
        if let Err(err) = deleted {
            tracing::warn!(%err, "could not remove the meeting of a failed start");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Ends the recording if the recorder dies by itself (device gone, disk full).
    fn spawn_watchdog(&self, meeting_id: String) {
        let shared = Arc::clone(&self.0);
        let spawned = std::thread::Builder::new()
            .name("recording-watchdog".into())
            .spawn(move || loop {
                std::thread::sleep(WATCHDOG_INTERVAL);
                let Ok(mut inner) = shared.lock() else { return };
                let Some(active) = inner.active.as_ref() else {
                    return;
                };
                if active.meeting.id != meeting_id {
                    return;
                }
                if active.recorder.is_running() {
                    continue;
                }
                let Some(active) = inner.active.take() else {
                    return;
                };
                let state = shared.finish(active, true);
                inner.last = state.clone();
                drop(inner);
                shared.events.state(&state);
                return;
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the recording watchdog");
        }
    }
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, AppError> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("recording state lock poisoned"))
    }

    /// Stops the recorder and closes the meeting. `died` is set when the watchdog found
    /// the recorder already gone, so the meeting is `interrupted` even without an error.
    fn finish(&self, active: Active, died: bool) -> RecordingState {
        let elapsed = active.elapsed();
        let meeting_id = active.meeting.id.clone();
        let result = active.recorder.stop();
        let (status, duration_ms, error) = match result {
            Ok(summary) if !summary.capture_lost && !died => {
                (MeetingStatus::Processing, summary.duration_ms, None)
            }
            Ok(summary) => (
                MeetingStatus::Interrupted,
                summary.duration_ms,
                Some(AppError::from(CaptureError::Stream("capture lost".into())).to_string()),
            ),
            Err(err) => (
                MeetingStatus::Interrupted,
                duration_ms(elapsed),
                Some(AppError::from(err).to_string()),
            ),
        };
        let duration_ms_i = i64::try_from(duration_ms).unwrap_or(i64::MAX);
        let closed = self.store.conn().map_err(AppError::from).and_then(|conn| {
            Ok(MeetingRepo::new(&conn).finish(
                &meeting_id,
                now_ms(),
                (duration_ms_i + 500) / 1_000,
                status,
            )?)
        });
        if let Err(err) = closed {
            // Startup recovery closes it next time; the audio is on disk either way.
            tracing::warn!(%err, "could not close the meeting");
        }
        tracing::info!(status = status.as_str(), duration_ms, "recording stopped");
        RecordingState {
            meeting_id: Some(meeting_id),
            state: Phase::Stopped,
            elapsed_ms: duration_ms,
            error,
        }
    }
}

fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::fake::FakeBackend;
    use crate::capture::recorder::tests::TempDir;
    use crate::capture::Track;

    #[derive(Default)]
    struct FakeEvents {
        states: Mutex<Vec<RecordingState>>,
        highlights: Mutex<Vec<Highlight>>,
    }

    impl FakeEvents {
        fn phases(&self) -> Vec<Phase> {
            self.states
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.state)
                .collect()
        }
        fn last(&self) -> RecordingState {
            self.states.lock().unwrap().last().cloned().unwrap()
        }
    }

    impl RecordingEvents for FakeEvents {
        fn state(&self, state: &RecordingState) {
            self.states.lock().unwrap().push(state.clone());
        }
        fn levels(&self, _levels: Levels) {}
        fn silent(&self, _meeting_id: &str) {}
        fn highlight(&self, highlight: &Highlight) {
            self.highlights.lock().unwrap().push(highlight.clone());
        }
    }

    fn service(backend: FakeBackend, dir: &TempDir) -> (RecordingService, Arc<FakeEvents>) {
        let events = Arc::new(FakeEvents::default());
        let service = RecordingService::new(
            Box::new(backend),
            Arc::new(Store::open_in_memory().unwrap()),
            dir.0.clone(),
            events.clone(),
        );
        (service, events)
    }

    fn live() -> FakeBackend {
        FakeBackend {
            script: vec![
                (Track::Mic, vec![0.1; 4_800]),
                (Track::Sys, vec![0.1; 4_800]),
            ],
            hold_open: true,
            ..FakeBackend::default()
        }
    }

    fn status(service: &RecordingService, id: &str) -> Option<String> {
        MeetingRepo::new(&service.0.store.conn().unwrap())
            .status(id)
            .unwrap()
    }

    #[test]
    fn fr_1_4_start_pause_resume_stop() {
        let dir = TempDir::new("service");
        let (service, events) = service(live(), &dir);
        assert_eq!(service.state().unwrap().state, Phase::Idle);

        let meeting = service.start(Some("Standup".into()), None).unwrap();
        assert_eq!(
            (
                meeting.title.as_str(),
                meeting.source_app.as_str(),
                meeting.status.as_str()
            ),
            ("Standup", "other", "recording")
        );
        assert_eq!(service.state().unwrap().state, Phase::Recording);
        assert_eq!(service.pause().unwrap().state, Phase::Paused);
        assert_eq!(service.resume().unwrap().state, Phase::Recording);
        let stopped = service.stop().unwrap();
        assert_eq!(stopped.state, Phase::Stopped);
        assert_eq!(stopped.error, None);
        assert_eq!(stopped.meeting_id.as_deref(), Some(meeting.id.as_str()));
        assert_eq!(service.state().unwrap(), stopped);
        assert_eq!(
            events.phases(),
            vec![
                Phase::Recording,
                Phase::Paused,
                Phase::Recording,
                Phase::Stopped
            ]
        );
        assert_eq!(status(&service, &meeting.id).as_deref(), Some("processing"));
        assert!(dir
            .0
            .join("meetings")
            .join(&meeting.id)
            .join("mic")
            .join("000001.opus.enc")
            .exists());
    }

    #[test]
    fn fr_9_1_highlights_mark_the_recording_while_it_runs_or_pauses() {
        let dir = TempDir::new("service-highlight");
        let (service, events) = service(live(), &dir);
        let idle = service.add_highlight(None).unwrap_err();
        assert_eq!(idle.code(), "invalid_state");

        let meeting = service.start(None, None).unwrap();
        let first = service.add_highlight(Some("  pricing ")).unwrap();
        assert_eq!(
            (first.meeting_id.as_str(), first.note.as_deref()),
            (meeting.id.as_str(), Some("pricing"))
        );
        service.pause().unwrap();
        let paused = service.add_highlight(Some("  ")).unwrap();
        assert_eq!(paused.note, None);
        assert!(paused.at_ms >= first.at_ms);
        let long = "x".repeat(501);
        assert_eq!(
            service.add_highlight(Some(&long)).unwrap_err().code(),
            "invalid_state"
        );
        service.stop().unwrap();
        assert_eq!(
            service.add_highlight(None).unwrap_err().code(),
            "invalid_state"
        );

        assert_eq!(*events.highlights.lock().unwrap(), [first, paused]);
        let saved = HighlightRepo::new(&service.0.store.conn().unwrap())
            .list(&meeting.id)
            .unwrap();
        assert_eq!(saved.len(), 2);
    }

    #[test]
    fn fr_1_3_untitled_recording_of_a_detected_app_is_named_after_it() {
        let dir = TempDir::new("service-title");
        let (service, _) = service(live(), &dir);
        let titled = |source_app: Option<&str>| {
            let meeting = service.start(None, source_app.map(Into::into)).unwrap();
            service.stop().unwrap();
            meeting.title
        };
        assert_eq!(titled(Some("zoom")), "Zoom meeting");
        assert_eq!(titled(Some("other")), "Recording");
        assert_eq!(titled(None), "Recording");
    }

    #[test]
    fn fr_1_6_active_names_the_recording_and_its_app() {
        let dir = TempDir::new("service-active");
        let (service, _) = service(live(), &dir);
        assert_eq!(service.active(), None);
        let meeting = service.start(None, Some("slack".into())).unwrap();
        let expected = Some(ActiveRecording {
            meeting_id: meeting.id,
            source_app: Some(SourceApp::Slack),
        });
        assert_eq!(service.active(), expected);
        service.pause().unwrap();
        assert_eq!(service.active(), expected, "paused still counts");
        service.stop().unwrap();
        assert_eq!(service.active(), None);
    }

    #[test]
    fn fr_1_6_two_quiet_minutes_report_once() {
        let quiet = Levels {
            mic_db: Some(-70.0),
            sys_db: None,
        };
        let loud = Levels {
            mic_db: Some(-20.0),
            sys_db: Some(-90.0),
        };
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut tracker = SilenceTracker::default();
        let mut fired = Vec::new();
        // 100 ms updates: quiet for 130 s, sound, then quiet again for 130 s.
        for n in 0..=1_300u64 {
            if tracker.update(quiet, ms(n * 100)) {
                fired.push(n * 100);
            }
        }
        assert!(!tracker.update(loud, ms(130_100)));
        for n in 1_302..=2_602u64 {
            if tracker.update(quiet, ms(n * 100)) {
                fired.push(n * 100);
            }
        }
        assert_eq!(fired, [120_000, 250_200]);
    }

    #[test]
    fn fr_1_6_paused_time_is_not_quiet_time() {
        let quiet = Levels {
            mic_db: None,
            sys_db: None,
        };
        let step = Duration::from_millis(100);
        let mut t = Instant::now();
        let mut tracker = SilenceTracker::default();
        let mut fired = false;
        // 100 s quiet, a 5 minute pause (no updates), then quiet again.
        for _ in 0..1_000 {
            fired |= tracker.update(quiet, t);
            t += step;
        }
        t += Duration::from_secs(300);
        for _ in 0..1_200 {
            fired |= tracker.update(quiet, t);
            t += step;
        }
        assert!(!fired, "quiet before the pause does not count");
        assert!(tracker.update(quiet, t));
    }

    #[test]
    fn actions_out_of_order_are_invalid_state() {
        let dir = TempDir::new("service-order");
        let (service, _) = service(live(), &dir);
        let code = |r: Result<RecordingState, AppError>| r.unwrap_err().code();
        assert_eq!(code(service.stop()), "invalid_state");
        assert_eq!(code(service.pause()), "invalid_state");
        service.start(None, None).unwrap();
        assert_eq!(code(service.resume()), "invalid_state");
        assert_eq!(
            service.start(None, None).unwrap_err().code(),
            "invalid_state"
        );
        service.pause().unwrap();
        assert_eq!(code(service.pause()), "invalid_state");
        service.shutdown();
        assert_eq!(service.state().unwrap().state, Phase::Stopped);
        service.shutdown();
    }

    #[test]
    fn failed_start_leaves_no_meeting() {
        let dir = TempDir::new("service-fail");
        let backend = FakeBackend {
            fail_start: true,
            ..FakeBackend::default()
        };
        let (service, events) = service(backend, &dir);
        assert_eq!(
            service.start(None, None).unwrap_err().code(),
            "audio_device"
        );
        let count: i64 = service
            .0
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM meetings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(events.phases().is_empty());
        assert_eq!(service.state().unwrap().state, Phase::Idle);
    }

    #[test]
    fn nfr_12_watchdog_ends_a_recording_whose_capture_died() {
        let dir = TempDir::new("service-died");
        // Not held open: the session ends by itself, like a vanished audio service.
        let backend = FakeBackend {
            script: vec![(Track::Mic, vec![0.1; 4_800])],
            ..FakeBackend::default()
        };
        let (service, events) = service(backend, &dir);
        let meeting = service.start(None, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while events.phases().last() != Some(&Phase::Stopped) {
            assert!(Instant::now() < deadline, "watchdog did not fire");
            std::thread::sleep(Duration::from_millis(20));
        }
        let last = events.last();
        assert!(last.error.is_some());
        assert_eq!(
            status(&service, &meeting.id).as_deref(),
            Some("interrupted")
        );
        assert_eq!(service.state().unwrap(), last);
    }
}
