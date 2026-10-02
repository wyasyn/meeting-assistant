//! End of meeting while recording (FR-1.6, ADR-018), as a pure state machine like `Engine`.
//! The recording's app is watched; a recording with no app (started by hand) watches every
//! known app that holds the mic while it runs. An app that closes, or that has held no mic
//! stream for `MIC_RELEASE_GRACE`, ends the meeting. Silence is the recorder's part.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::apps::SourceApp;
use super::ActiveRecording;

/// Mic released this long ends the meeting; shorter gaps are mute toggles or device switches.
pub const MIC_RELEASE_GRACE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    AppClosed,
    MicReleased,
    /// Both tracks quiet for 2 minutes; reported by the recording, not here.
    Silence,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppClosed => "app_closed",
            Self::MicReleased => "mic_released",
            Self::Silence => "silence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub app: SourceApp,
    pub reason: EndReason,
}

#[derive(Debug, Default)]
struct Watched {
    /// Its process was seen during this recording, so its absence means it closed.
    seen_running: bool,
    released_at: Option<Instant>,
    /// Reported for this release; reset when it holds the mic again.
    reported: bool,
}

#[derive(Debug, Default)]
pub struct EndWatch {
    meeting_id: Option<String>,
    apps: HashMap<SourceApp, Watched>,
}

impl EndWatch {
    /// Advances to `now`. `holding` are the apps holding a mic stream, `running` the apps
    /// whose process is up.
    pub fn tick(
        &mut self,
        now: Instant,
        recording: Option<&ActiveRecording>,
        holding: &HashSet<SourceApp>,
        running: &HashSet<SourceApp>,
    ) -> Vec<Ended> {
        let Some(recording) = recording else {
            self.meeting_id = None;
            self.apps.clear();
            return Vec::new();
        };
        if self.meeting_id.as_deref() != Some(recording.meeting_id.as_str()) {
            self.meeting_id = Some(recording.meeting_id.clone());
            self.apps.clear();
            if let Some(app) = recording.source_app {
                self.apps.insert(app, Watched::default());
            }
        }
        if recording.source_app.is_none() {
            for app in holding {
                self.apps.entry(*app).or_default();
            }
        }

        let mut out = Vec::new();
        for app in SourceApp::ALL {
            let Some(watched) = self.apps.get_mut(&app) else {
                continue;
            };
            let runs = running.contains(&app);
            watched.seen_running |= runs;
            if holding.contains(&app) {
                watched.released_at = None;
                watched.reported = false;
                continue;
            }
            let released = *watched.released_at.get_or_insert(now);
            if watched.reported {
                continue;
            }
            let reason = if watched.seen_running && !runs {
                EndReason::AppClosed
            } else if now.saturating_duration_since(released) >= MIC_RELEASE_GRACE {
                EndReason::MicReleased
            } else {
                continue;
            };
            watched.reported = true;
            out.push(Ended { app, reason });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, app: Option<SourceApp>) -> ActiveRecording {
        ActiveRecording {
            meeting_id: id.into(),
            source_app: app,
        }
    }

    fn set(apps: &[SourceApp]) -> HashSet<SourceApp> {
        apps.iter().copied().collect()
    }

    fn at(t0: Instant, s: u64) -> Instant {
        t0 + Duration::from_secs(s)
    }

    use SourceApp::{Browser, Slack, Zoom};

    #[test]
    fn fr_1_6_app_closed_is_reported_once() {
        let t0 = Instant::now();
        let mut watch = EndWatch::default();
        let r = rec("m1", Some(Zoom));
        assert!(watch
            .tick(t0, Some(&r), &set(&[Zoom]), &set(&[Zoom]))
            .is_empty());
        assert_eq!(
            watch.tick(at(t0, 3), Some(&r), &set(&[]), &set(&[])),
            [Ended {
                app: Zoom,
                reason: EndReason::AppClosed
            }]
        );
        for s in 4..200 {
            assert!(watch
                .tick(at(t0, s), Some(&r), &set(&[]), &set(&[]))
                .is_empty());
        }
    }

    #[test]
    fn fr_1_6_mic_released_after_the_grace_only() {
        let t0 = Instant::now();
        let mut watch = EndWatch::default();
        let r = rec("m1", Some(Browser));
        let up = set(&[Browser]);
        watch.tick(t0, Some(&r), &up, &up);
        // Muted for 20 s, then back: not an end.
        for s in 1..=20 {
            assert!(watch.tick(at(t0, s), Some(&r), &set(&[]), &up).is_empty());
        }
        watch.tick(at(t0, 21), Some(&r), &up, &up);
        // Left the call; the browser stays open.
        for s in 22..52 {
            assert!(watch.tick(at(t0, s), Some(&r), &set(&[]), &up).is_empty());
        }
        assert_eq!(
            watch.tick(at(t0, 52), Some(&r), &set(&[]), &up),
            [Ended {
                app: Browser,
                reason: EndReason::MicReleased
            }]
        );
        assert!(watch.tick(at(t0, 90), Some(&r), &set(&[]), &up).is_empty());
        // A new stretch with the mic re-arms it.
        watch.tick(at(t0, 100), Some(&r), &up, &up);
        assert_eq!(watch.tick(at(t0, 130), Some(&r), &set(&[]), &up).len(), 1);
    }

    #[test]
    fn fr_1_6_app_never_seen_running_ends_by_mic_release() {
        // Flatpak apps: the stream names them, their process is hidden.
        let t0 = Instant::now();
        let mut watch = EndWatch::default();
        let r = rec("m1", Some(Slack));
        watch.tick(t0, Some(&r), &set(&[Slack]), &set(&[]));
        assert!(watch
            .tick(at(t0, 29), Some(&r), &set(&[]), &set(&[]))
            .is_empty());
        assert_eq!(
            watch.tick(at(t0, 31), Some(&r), &set(&[]), &set(&[]))[0].reason,
            EndReason::MicReleased
        );
    }

    #[test]
    fn fr_1_6_manual_recording_watches_apps_that_join() {
        let t0 = Instant::now();
        let mut watch = EndWatch::default();
        let r = rec("m1", None);
        // No meeting app yet: nothing to end.
        for s in 0..100 {
            assert!(watch
                .tick(at(t0, s), Some(&r), &set(&[]), &set(&[]))
                .is_empty());
        }
        watch.tick(at(t0, 100), Some(&r), &set(&[Zoom]), &set(&[Zoom]));
        assert_eq!(
            watch.tick(at(t0, 101), Some(&r), &set(&[]), &set(&[])),
            [Ended {
                app: Zoom,
                reason: EndReason::AppClosed
            }]
        );
    }

    #[test]
    fn a_new_recording_starts_fresh_and_no_recording_reports_nothing() {
        let t0 = Instant::now();
        let mut watch = EndWatch::default();
        let r1 = rec("m1", Some(Zoom));
        watch.tick(t0, Some(&r1), &set(&[Zoom]), &set(&[Zoom]));
        assert!(watch.tick(at(t0, 1), None, &set(&[]), &set(&[])).is_empty());
        let r2 = rec("m2", Some(Slack));
        // Zoom closing no longer matters; Slack is watched.
        assert!(watch
            .tick(at(t0, 2), Some(&r2), &set(&[Slack]), &set(&[Slack]))
            .is_empty());
        assert_eq!(
            watch.tick(at(t0, 3), Some(&r2), &set(&[]), &set(&[]))[0].app,
            Slack
        );
    }

    #[test]
    fn reasons_use_snake_case_on_the_wire() {
        for reason in [
            EndReason::AppClosed,
            EndReason::MicReleased,
            EndReason::Silence,
        ] {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::Value::String(reason.as_str().into())
            );
        }
    }
}
