//! Detection rules as a pure state machine (FR-1.1, FR-1.2, ADR-016). No threads, no clock:
//! callers pass the time, so every rule is table-tested.
//!
//! A known app holding a mic stream long enough is a meeting. Each app signals once per
//! meeting and re-arms only after it has held no mic stream for `REARM_AFTER`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::apps::{from_process_name, SourceApp};
use super::{ProcessInfo, StreamEvent};

/// How long a desktop app must hold the mic.
pub const APP_HOLD: Duration = Duration::from_secs(3);
/// How long a browser must hold the mic; filters voice search and quick mic tests.
pub const BROWSER_HOLD: Duration = Duration::from_secs(10);
/// Mic released this long means the meeting is over and the app may signal again.
pub const REARM_AFTER: Duration = Duration::from_secs(60);

pub const CONFIDENCE_APP_RUNNING: f64 = 0.9;
/// The stream names the app but its process is not visible (for example a Flatpak).
pub const CONFIDENCE_STREAM_ONLY: f64 = 0.7;
pub const CONFIDENCE_BROWSER: f64 = 0.5;

/// What the engine decided: a meeting in `app`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    pub app: SourceApp,
    pub confidence: f64,
}

#[derive(Debug)]
struct Stream {
    pid: Option<u32>,
    /// From the stream's binary name; wins over the pid lookup.
    named: Option<SourceApp>,
}

#[derive(Debug, Default)]
struct AppState {
    holding_since: Option<Instant>,
    released_at: Option<Instant>,
    signalled: bool,
}

#[derive(Debug)]
pub struct Engine {
    own_pid: u32,
    streams: HashMap<u32, Stream>,
    /// Known apps by pid, from the last process scan.
    pids: HashMap<u32, SourceApp>,
    running: HashSet<SourceApp>,
    apps: HashMap<SourceApp, AppState>,
}

impl Engine {
    /// `own_pid` is this app's process; its own capture streams never count.
    pub fn new(own_pid: u32) -> Self {
        Self {
            own_pid,
            streams: HashMap::new(),
            pids: HashMap::new(),
            running: HashSet::new(),
            apps: HashMap::new(),
        }
    }

    pub fn set_processes(&mut self, processes: &[ProcessInfo]) {
        self.pids = processes
            .iter()
            .filter_map(|p| from_process_name(&p.name).map(|app| (p.pid, app)))
            .collect();
        self.running = self.pids.values().copied().collect();
    }

    pub fn stream(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Opened { id, info } => {
                if info.pid == Some(self.own_pid) {
                    return;
                }
                let named = info.binary.as_deref().and_then(from_process_name);
                self.streams.insert(
                    id,
                    Stream {
                        pid: info.pid,
                        named,
                    },
                );
            }
            StreamEvent::Closed { id } => {
                self.streams.remove(&id);
            }
        }
    }

    /// Forgets all streams, for when the stream watch restarts and reports them again.
    pub fn clear_streams(&mut self) {
        self.streams.clear();
    }

    fn app_of(&self, stream: &Stream) -> Option<SourceApp> {
        stream
            .named
            .or_else(|| stream.pid.and_then(|pid| self.pids.get(&pid).copied()))
    }

    /// Known apps holding a mic stream now.
    pub fn holding(&self) -> HashSet<SourceApp> {
        self.streams
            .values()
            .filter_map(|s| self.app_of(s))
            .collect()
    }

    /// Known apps whose process was in the last scan.
    pub fn running(&self) -> &HashSet<SourceApp> {
        &self.running
    }

    /// Advances to `now`. While `recording`, nothing is signalled and apps holding the mic
    /// count as signalled, so stopping mid-call does not prompt for the same meeting.
    pub fn tick(&mut self, now: Instant, recording: bool) -> Vec<Detection> {
        let holding = self.holding();
        let mut out = Vec::new();
        for app in SourceApp::ALL {
            let state = self.apps.entry(app).or_default();
            if holding.contains(&app) {
                state.released_at = None;
                let since = *state.holding_since.get_or_insert(now);
                if recording {
                    state.signalled = true;
                    continue;
                }
                let hold = match app {
                    SourceApp::Browser => BROWSER_HOLD,
                    _ => APP_HOLD,
                };
                if !state.signalled && now.saturating_duration_since(since) >= hold {
                    state.signalled = true;
                    let confidence = match app {
                        SourceApp::Browser => CONFIDENCE_BROWSER,
                        _ if self.running.contains(&app) => CONFIDENCE_APP_RUNNING,
                        _ => CONFIDENCE_STREAM_ONLY,
                    };
                    out.push(Detection { app, confidence });
                }
            } else {
                state.holding_since = None;
                let released = *state.released_at.get_or_insert(now);
                if state.signalled && now.saturating_duration_since(released) >= REARM_AFTER {
                    state.signalled = false;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::StreamInfo;

    const OWN_PID: u32 = 1;

    fn opened(id: u32, pid: Option<u32>, binary: Option<&str>) -> StreamEvent {
        StreamEvent::Opened {
            id,
            info: StreamInfo {
                pid,
                binary: binary.map(str::to_owned),
            },
        }
    }

    fn process(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
        }
    }

    fn secs(t0: Instant, s: f64) -> Instant {
        t0 + Duration::from_secs_f64(s)
    }

    /// Ticks once per `step` seconds from `from` to `to` (inclusive) and collects detections.
    fn run(
        engine: &mut Engine,
        t0: Instant,
        from: f64,
        to: f64,
        recording: bool,
    ) -> Vec<Detection> {
        let mut out = Vec::new();
        let mut s = from;
        while s <= to + 1e-9 {
            out.extend(engine.tick(secs(t0, s), recording));
            s += 0.5;
        }
        out
    }

    #[test]
    fn fr_1_1_app_signals_once_after_holding_the_mic_for_3_s() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(10, "zoom")]);
        engine.stream(opened(100, Some(10), Some("zoom")));
        assert!(engine.tick(t0, false).is_empty());
        assert!(engine.tick(secs(t0, 2.9), false).is_empty());
        assert_eq!(
            engine.tick(secs(t0, 3.0), false),
            vec![Detection {
                app: SourceApp::Zoom,
                confidence: CONFIDENCE_APP_RUNNING
            }]
        );
        assert!(run(&mut engine, t0, 3.5, 600.0, false).is_empty());
    }

    #[test]
    fn fr_1_2_browser_needs_10_s_and_gets_low_confidence() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(20, "firefox")]);
        engine.stream(opened(200, Some(20), Some("firefox")));
        assert!(run(&mut engine, t0, 0.0, 9.5, false).is_empty());
        assert_eq!(
            engine.tick(secs(t0, 10.0), false),
            vec![Detection {
                app: SourceApp::Browser,
                confidence: CONFIDENCE_BROWSER
            }]
        );
    }

    #[test]
    fn fr_1_1_confidence_depends_on_the_process_being_seen() {
        let cases = [
            (vec![process(10, "slack")], CONFIDENCE_APP_RUNNING),
            (vec![], CONFIDENCE_STREAM_ONLY),
        ];
        for (processes, confidence) in cases {
            let t0 = Instant::now();
            let mut engine = Engine::new(OWN_PID);
            engine.set_processes(&processes);
            engine.stream(opened(100, Some(10), Some("slack")));
            let got = run(&mut engine, t0, 0.0, 3.0, false);
            assert_eq!(
                got,
                vec![Detection {
                    app: SourceApp::Slack,
                    confidence
                }]
            );
        }
    }

    #[test]
    fn fr_1_1_stream_without_binary_is_resolved_by_pid() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.stream(opened(100, Some(30), None));
        assert!(run(&mut engine, t0, 0.0, 5.0, false).is_empty());
        // The next process scan names the pid; the hold counts from then.
        engine.set_processes(&[process(30, "Discord")]);
        assert!(engine.tick(secs(t0, 5.5), false).is_empty());
        assert_eq!(
            engine.tick(secs(t0, 8.5), false),
            vec![Detection {
                app: SourceApp::Discord,
                confidence: CONFIDENCE_APP_RUNNING
            }]
        );
    }

    #[test]
    fn fr_1_1_debounce_rearms_only_after_60_s_without_the_mic() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(10, "zoom")]);
        engine.stream(opened(100, Some(10), Some("zoom")));
        assert_eq!(run(&mut engine, t0, 0.0, 10.0, false).len(), 1);

        // Mute toggle or device switch: closed for 30 s, then back.
        engine.stream(StreamEvent::Closed { id: 100 });
        assert!(run(&mut engine, t0, 10.5, 40.0, false).is_empty());
        engine.stream(opened(101, Some(10), Some("zoom")));
        assert!(run(&mut engine, t0, 40.5, 100.0, false).is_empty());

        // Left the call: 60 s without the mic, then a new call.
        engine.stream(StreamEvent::Closed { id: 101 });
        assert!(run(&mut engine, t0, 100.5, 160.5, false).is_empty());
        engine.stream(opened(102, Some(10), Some("zoom")));
        assert_eq!(run(&mut engine, t0, 161.0, 170.0, false).len(), 1);
    }

    #[test]
    fn fr_1_1_two_streams_from_one_app_count_once() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(10, "teams-for-linux")]);
        engine.stream(opened(100, Some(10), Some("teams-for-linux")));
        engine.stream(opened(101, Some(10), Some("teams-for-linux")));
        assert_eq!(run(&mut engine, t0, 0.0, 30.0, false).len(), 1);
        // One of them closing does not end the meeting.
        engine.stream(StreamEvent::Closed { id: 100 });
        assert!(run(&mut engine, t0, 30.5, 200.0, false).is_empty());
    }

    #[test]
    fn fr_1_3_nothing_is_signalled_while_recording_or_right_after_a_mid_call_stop() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(10, "zoom")]);
        engine.stream(opened(100, Some(10), Some("zoom")));
        assert!(run(&mut engine, t0, 0.0, 30.0, true).is_empty());
        // Recording stopped, call still running.
        assert!(run(&mut engine, t0, 30.5, 120.0, false).is_empty());
    }

    #[test]
    fn fr_1_1_unknown_apps_and_our_own_streams_are_ignored() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.set_processes(&[process(OWN_PID, "zoom"), process(40, "obs")]);
        engine.stream(opened(100, Some(OWN_PID), Some("zoom")));
        engine.stream(opened(101, Some(40), Some("obs")));
        engine.stream(opened(102, None, None));
        assert!(run(&mut engine, t0, 0.0, 30.0, false).is_empty());
    }

    #[test]
    fn clearing_streams_releases_the_mic() {
        let t0 = Instant::now();
        let mut engine = Engine::new(OWN_PID);
        engine.stream(opened(100, Some(10), Some("zoom")));
        assert!(engine.tick(t0, false).is_empty());
        engine.clear_streams();
        assert!(run(&mut engine, t0, 0.5, 10.0, false).is_empty());
    }
}
