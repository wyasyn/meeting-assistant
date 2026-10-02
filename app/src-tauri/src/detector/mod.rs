//! Meeting detection (FR-1.1, FR-1.2, ADR-016): process watch plus mic streams, and later
//! extension messages and the calendar. Detection never records: it only emits
//! `meeting:detected`, and recording still starts from a user action (rule 1).

pub mod apps;
pub mod engine;
#[cfg(target_os = "linux")]
mod pipewire;
mod processes;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use apps::SourceApp;
use engine::Engine;

const TICK: Duration = Duration::from_secs(1);
const PROCESS_INTERVAL: Duration = Duration::from_secs(3);
/// Wait before reopening the stream watch, for example after PipeWire restarted.
const WATCH_RETRY: Duration = Duration::from_secs(10);

/// Payload of `meeting:detected`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetected {
    pub signal_id: String,
    pub source_app: SourceApp,
    /// Unknown until the extension or the calendar supply it; Wayland hides window titles.
    pub title: Option<String>,
    /// 0 to 1.
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

/// What a mic (capture) stream says about its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub pid: Option<u32>,
    pub binary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Opened { id: u32, info: StreamInfo },
    Closed { id: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum DetectorError {
    /// Built only on systems without a stream watch yet.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    #[error("meeting detection is not supported on this system yet")]
    Unsupported,
    #[error("could not watch audio streams: {0}")]
    Watch(String),
}

pub trait ProcessSource: Send {
    fn list(&mut self) -> Vec<ProcessInfo>;
}

/// Reports mic streams opening and closing until the returned watch is dropped. When the
/// watch ends by itself the sender is dropped, so the receiver sees a disconnect.
pub trait MicStreamSource: Send {
    fn watch(&self, events: Sender<StreamEvent>) -> Result<StreamWatch, DetectorError>;
}

/// A running stream watch; stops and joins its thread on drop.
pub struct StreamWatch {
    stop: Option<Box<dyn FnOnce() + Send>>,
    thread: Option<JoinHandle<()>>,
}

impl StreamWatch {
    pub fn new(stop: Box<dyn FnOnce() + Send>, thread: Option<JoinHandle<()>>) -> Self {
        Self {
            stop: Some(stop),
            thread,
        }
    }
}

impl Drop for StreamWatch {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::warn!("stream watch thread panicked");
            }
        }
    }
}

/// Where detections go: the `meeting:detected` event in the app, a fake in tests.
pub trait DetectorEvents: Send + Sync {
    fn detected(&self, signal: &MeetingDetected);
}

/// Mic streams on this OS.
pub fn default_streams() -> Box<dyn MicStreamSource> {
    #[cfg(target_os = "linux")]
    {
        Box::new(pipewire::PipeWireStreams::new())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(Unsupported)
    }
}

pub fn default_processes() -> Box<dyn ProcessSource> {
    Box::new(processes::SysinfoProcesses::new())
}

#[cfg(not(target_os = "linux"))]
struct Unsupported;

#[cfg(not(target_os = "linux"))]
impl MicStreamSource for Unsupported {
    fn watch(&self, _events: Sender<StreamEvent>) -> Result<StreamWatch, DetectorError> {
        Err(DetectorError::Unsupported)
    }
}

/// One detection step at a time; the service thread drives it every `TICK`.
struct Worker {
    engine: Engine,
    processes: Box<dyn ProcessSource>,
    streams: Box<dyn MicStreamSource>,
    events: Arc<dyn DetectorEvents>,
    is_recording: Box<dyn Fn() -> bool + Send>,
    watch: Option<(StreamWatch, Receiver<StreamEvent>)>,
    next_watch: Option<Instant>,
    next_scan: Option<Instant>,
}

impl Worker {
    fn new(
        processes: Box<dyn ProcessSource>,
        streams: Box<dyn MicStreamSource>,
        events: Arc<dyn DetectorEvents>,
        is_recording: Box<dyn Fn() -> bool + Send>,
    ) -> Self {
        Self {
            engine: Engine::new(std::process::id()),
            processes,
            streams,
            events,
            is_recording,
            watch: None,
            next_watch: None,
            next_scan: None,
        }
    }

    /// Returns false when detection cannot run on this system at all.
    fn step(&mut self, now: Instant) -> bool {
        if self.watch.is_none() && self.next_watch.is_none_or(|at| now >= at) {
            let (tx, rx) = mpsc::channel();
            match self.streams.watch(tx) {
                Ok(watch) => {
                    tracing::info!("meeting detection started");
                    self.watch = Some((watch, rx));
                }
                Err(DetectorError::Unsupported) => {
                    tracing::info!("meeting detection is not available on this system");
                    return false;
                }
                Err(err) => {
                    tracing::warn!(%err, "could not start meeting detection, retrying");
                    self.next_watch = Some(now + WATCH_RETRY);
                }
            }
        }

        if self.next_scan.is_none_or(|at| now >= at) {
            self.engine.set_processes(&self.processes.list());
            self.next_scan = Some(now + PROCESS_INTERVAL);
        }

        if let Some((_, rx)) = &self.watch {
            let mut ended = false;
            loop {
                match rx.try_recv() {
                    Ok(event) => self.engine.stream(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
            if ended {
                tracing::warn!("stream watch ended, retrying");
                self.watch = None;
                self.engine.clear_streams();
                self.next_watch = Some(now + WATCH_RETRY);
            }
        }

        for detection in self.engine.tick(now, (self.is_recording)()) {
            let signal = MeetingDetected {
                signal_id: uuid::Uuid::now_v7().to_string(),
                source_app: detection.app,
                title: None,
                confidence: detection.confidence,
            };
            tracing::info!(
                source_app = signal.source_app.as_str(),
                confidence = signal.confidence,
                "meeting detected"
            );
            self.events.detected(&signal);
        }
        true
    }
}

/// Runs detection on its own thread until `shutdown`.
pub struct DetectorService {
    stop: Mutex<Option<Sender<()>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl DetectorService {
    pub fn start(
        processes: Box<dyn ProcessSource>,
        streams: Box<dyn MicStreamSource>,
        events: Arc<dyn DetectorEvents>,
        is_recording: Box<dyn Fn() -> bool + Send>,
    ) -> Self {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let mut worker = Worker::new(processes, streams, events, is_recording);
        let thread = std::thread::Builder::new()
            .name("detector".into())
            .spawn(move || loop {
                if !worker.step(Instant::now()) {
                    return;
                }
                match stop_rx.recv_timeout(TICK) {
                    Err(RecvTimeoutError::Timeout) => {}
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(err) => {
                tracing::warn!(%err, "could not start the detector thread");
                None
            }
        };
        Self {
            stop: Mutex::new(Some(stop_tx)),
            thread: Mutex::new(thread),
        }
    }

    /// Stops the thread and its stream watch. Safe to call more than once.
    pub fn shutdown(&self) {
        if let Ok(mut stop) = self.stop.lock() {
            stop.take();
        }
        let thread = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(thread) = thread {
            if thread.join().is_err() {
                tracing::warn!("detector thread panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct FakeProcesses(Vec<ProcessInfo>);

    impl ProcessSource for FakeProcesses {
        fn list(&mut self) -> Vec<ProcessInfo> {
            self.0.clone()
        }
    }

    /// Hands the stream sender to the test so it can play PipeWire.
    struct FakeStreams {
        senders: Arc<Mutex<Vec<Sender<StreamEvent>>>>,
        fail: bool,
    }

    impl MicStreamSource for FakeStreams {
        fn watch(&self, events: Sender<StreamEvent>) -> Result<StreamWatch, DetectorError> {
            if self.fail {
                return Err(DetectorError::Watch("down".into()));
            }
            self.senders.lock().unwrap().push(events);
            Ok(StreamWatch::new(Box::new(|| {}), None))
        }
    }

    #[derive(Default)]
    struct FakeEvents(Mutex<Vec<MeetingDetected>>);

    impl DetectorEvents for FakeEvents {
        fn detected(&self, signal: &MeetingDetected) {
            self.0.lock().unwrap().push(signal.clone());
        }
    }

    type Senders = Arc<Mutex<Vec<Sender<StreamEvent>>>>;

    fn worker(recording: Arc<AtomicBool>, fail: bool) -> (Worker, Senders, Arc<FakeEvents>) {
        let senders = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(FakeEvents::default());
        let worker = Worker::new(
            Box::new(FakeProcesses(vec![ProcessInfo {
                pid: 4242,
                name: "zoom".into(),
            }])),
            Box::new(FakeStreams {
                senders: senders.clone(),
                fail,
            }),
            events.clone(),
            Box::new(move || recording.load(Ordering::SeqCst)),
        );
        (worker, senders, events)
    }

    fn zoom_stream(id: u32) -> StreamEvent {
        StreamEvent::Opened {
            id,
            info: StreamInfo {
                pid: Some(4242),
                binary: Some("zoom".into()),
            },
        }
    }

    #[test]
    fn fr_1_1_stream_events_end_in_one_meeting_detected() {
        let (mut worker, senders, events) = worker(Arc::default(), false);
        let t0 = Instant::now();
        assert!(worker.step(t0));
        senders.lock().unwrap()[0].send(zoom_stream(7)).unwrap();
        for s in 1..=20 {
            assert!(worker.step(t0 + Duration::from_secs(s)));
        }
        let got = events.0.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source_app, SourceApp::Zoom);
        assert_eq!(got[0].confidence, engine::CONFIDENCE_APP_RUNNING);
        assert_eq!(got[0].title, None);
        assert!(uuid::Uuid::parse_str(&got[0].signal_id).is_ok());
        let json = serde_json::to_value(&got[0]).unwrap();
        assert_eq!(json["sourceApp"], "zoom");
        assert!(json.get("signalId").is_some());
    }

    #[test]
    fn fr_1_3_no_signal_while_recording() {
        let recording = Arc::new(AtomicBool::new(true));
        let (mut worker, senders, events) = worker(recording, false);
        let t0 = Instant::now();
        worker.step(t0);
        senders.lock().unwrap()[0].send(zoom_stream(7)).unwrap();
        for s in 1..=20 {
            worker.step(t0 + Duration::from_secs(s));
        }
        assert!(events.0.lock().unwrap().is_empty());
    }

    #[test]
    fn ended_watch_is_reopened_and_its_streams_forgotten() {
        let (mut worker, senders, events) = worker(Arc::default(), false);
        let t0 = Instant::now();
        worker.step(t0);
        senders.lock().unwrap()[0].send(zoom_stream(7)).unwrap();
        // PipeWire went away before the hold was reached.
        senders.lock().unwrap().clear();
        worker.step(t0 + Duration::from_secs(1));
        assert!(worker.watch.is_none());
        for s in 2..10 {
            worker.step(t0 + Duration::from_secs(s));
        }
        assert!(events.0.lock().unwrap().is_empty());
        assert!(senders.lock().unwrap().is_empty());
        worker.step(t0 + Duration::from_secs(11));
        assert_eq!(senders.lock().unwrap().len(), 1);
    }

    #[test]
    fn failed_watch_is_retried_later() {
        let (mut worker, _senders, _events) = worker(Arc::default(), true);
        let t0 = Instant::now();
        assert!(worker.step(t0));
        assert!(worker.watch.is_none());
        assert_eq!(worker.next_watch, Some(t0 + WATCH_RETRY));
    }
}
