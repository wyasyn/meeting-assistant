//! Consent prompt and per-app rules (FR-1.3, FR-1.7, ADR-017). Every detection passes
//! through here: `never` drops it, `always` starts recording (the rule is the user's
//! action, rule 1) and `ask` shows the prompt, both as a notification with Record /
//! Not now / Never and as `meeting:detected` for the window. Whichever is answered first
//! wins; the other is closed.

#[cfg(target_os = "linux")]
mod desktop;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use crate::detector::apps::SourceApp;
use crate::detector::{DetectorEvents, MeetingDetected};
use crate::error::AppError;
use crate::recording::{Phase, RecordingService};
use crate::store::app_rules::{AppRule, AppRulesRepo};
use crate::store::Store;

/// What the user did with a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Record,
    NotNow,
    Never,
    Stop,
    /// Clicked the notification itself.
    Open,
    /// Closed without a choice: by the user, a timeout or `Notifier::close`.
    Dismissed,
}

impl Choice {
    /// Action key on the notification.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn key(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::NotNow => "not-now",
            Self::Never => "never",
            Self::Stop => "stop",
            Self::Open => "default",
            Self::Dismissed => "dismissed",
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn from_key(key: &str) -> Self {
        [
            Self::Record,
            Self::NotNow,
            Self::Never,
            Self::Stop,
            Self::Open,
        ]
        .into_iter()
        .find(|choice| choice.key() == key)
        .unwrap_or(Self::Dismissed)
    }
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub summary: String,
    pub body: String,
    pub buttons: Vec<(Choice, String)>,
    /// Stays until answered instead of expiring.
    pub sticky: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoteId(pub u32);

/// Runs once with the user's choice, on a thread where blocking is fine.
pub type OnChoice = Box<dyn FnOnce(Choice) + Send>;

/// OS notifications: the desktop notification service on Linux, a fake in tests.
pub trait Notifier: Send + Sync {
    /// Shows `note`; `None` when it could not be shown (the window prompt still is).
    fn show(&self, note: Note, on_choice: OnChoice) -> Option<NoteId>;
    /// Closes a note if it is still shown. Its `on_choice` then gets `Dismissed`.
    fn close(&self, id: NoteId);
}

/// The part of `RecordingService` consent needs; faked in tests.
pub trait RecordingControl: Send + Sync {
    fn start_for(&self, app: SourceApp) -> Result<(), AppError>;
    fn stop(&self) -> Result<(), AppError>;
}

impl RecordingControl for RecordingService {
    fn start_for(&self, app: SourceApp) -> Result<(), AppError> {
        // No title: the service names it after the app.
        self.start(None, Some(app.as_str().into())).map(drop)
    }

    fn stop(&self) -> Result<(), AppError> {
        RecordingService::stop(self).map(drop)
    }
}

/// Where prompts go in the window: Tauri events in the app, a fake in tests.
pub trait ConsentEvents: Send + Sync {
    /// `meeting:detected`: the window shows its prompt.
    fn prompt(&self, signal: &MeetingDetected);
    /// `meeting:prompt-closed`: the prompt was answered or is no longer needed.
    fn prompt_closed(&self, signal_id: &str);
    fn show_window(&self);
}

/// One row of `list_app_rules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRuleEntry {
    pub source_app: SourceApp,
    pub rule: AppRule,
}

/// Notifications on this OS. Other OSes have no detection yet (ADR-016), so the window
/// prompt is enough there for now.
pub fn default_notifier(app_name: String) -> Box<dyn Notifier> {
    #[cfg(target_os = "linux")]
    {
        Box::new(desktop::DesktopNotifier::new(app_name))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app_name;
        Box::new(NoNotifier)
    }
}

#[cfg(not(target_os = "linux"))]
struct NoNotifier;

#[cfg(not(target_os = "linux"))]
impl Notifier for NoNotifier {
    fn show(&self, _note: Note, _on_choice: OnChoice) -> Option<NoteId> {
        None
    }
    fn close(&self, _id: NoteId) {}
}

struct Prompt {
    app: SourceApp,
    /// `None` once the notification is gone; the window prompt may still be open.
    note: Option<NoteId>,
}

#[derive(Default)]
struct Open {
    /// By signal id.
    prompts: HashMap<String, Prompt>,
    /// "Recording" note with a Stop button, after a rule started the recording.
    rule_note: Option<NoteId>,
}

struct Shared {
    store: Arc<Store>,
    recorder: Arc<dyn RecordingControl>,
    notifier: Box<dyn Notifier>,
    events: Arc<dyn ConsentEvents>,
    open: Mutex<Open>,
}

/// Managed as Tauri state and handed to the detector as its `DetectorEvents`. Cheap to clone.
#[derive(Clone)]
pub struct ConsentService(Arc<Shared>);

impl ConsentService {
    pub fn new(
        store: Arc<Store>,
        recorder: Arc<dyn RecordingControl>,
        notifier: Box<dyn Notifier>,
        events: Arc<dyn ConsentEvents>,
    ) -> Self {
        Self(Arc::new(Shared {
            store,
            recorder,
            notifier,
            events,
            open: Mutex::new(Open::default()),
        }))
    }

    /// Every known app with its rule; `ask` when none is set.
    pub fn rules(&self) -> Result<Vec<AppRuleEntry>, AppError> {
        let conn = self.0.store.conn()?;
        let repo = AppRulesRepo::new(&conn);
        SourceApp::ALL
            .into_iter()
            .map(|app| {
                Ok(AppRuleEntry {
                    source_app: app,
                    rule: repo.get(app.as_str())?,
                })
            })
            .collect()
    }

    pub fn set_rule(&self, app: SourceApp, rule: AppRule) -> Result<(), AppError> {
        AppRulesRepo::new(&*self.0.store.conn()?).set(app.as_str(), rule)?;
        tracing::info!(
            source_app = app.as_str(),
            rule = rule.as_str(),
            "app rule changed"
        );
        if rule == AppRule::Never {
            self.close_prompts(|prompt| prompt.app == app);
        }
        Ok(())
    }

    /// Follows the recording state. A running recording makes open prompts moot, and the
    /// rule's Stop note goes once the recording ends.
    pub fn recording_changed(&self, phase: Phase) {
        match phase {
            Phase::Recording => self.close_prompts(|_| true),
            Phase::Stopped | Phase::Idle => {
                let note = self.open().rule_note.take();
                if let Some(note) = note {
                    self.0.notifier.close(note);
                }
            }
            Phase::Paused => {}
        }
    }

    fn on_detected(&self, signal: &MeetingDetected) {
        let app = signal.source_app;
        let rule = self
            .0
            .store
            .conn()
            .map_err(AppError::from)
            .and_then(|conn| Ok(AppRulesRepo::new(&conn).get(app.as_str())?))
            .unwrap_or_else(|err| {
                tracing::warn!(%err, "could not read the app rule, asking");
                AppRule::Ask
            });
        match rule {
            AppRule::Never => {
                tracing::info!(
                    source_app = app.as_str(),
                    "detection ignored by the app rule"
                );
            }
            AppRule::Always => match self.0.recorder.start_for(app) {
                Ok(()) => {
                    tracing::info!(
                        source_app = app.as_str(),
                        "recording started by the app rule"
                    );
                    self.show_rule_note(app);
                }
                Err(err) => {
                    tracing::warn!(%err, "could not start recording by the app rule, asking");
                    self.ask(signal);
                }
            },
            AppRule::Ask => self.ask(signal),
        }
    }

    fn ask(&self, signal: &MeetingDetected) {
        let app = signal.source_app;
        // One prompt per app: a newer detection replaces an unanswered one.
        self.close_prompts(|prompt| prompt.app == app);
        let signal_id = signal.signal_id.clone();
        self.open()
            .prompts
            .insert(signal_id.clone(), Prompt { app, note: None });
        self.0.events.prompt(signal);

        let service = self.clone();
        let answered_id = signal_id.clone();
        let note = self.0.notifier.show(
            prompt_note(app),
            Box::new(move |choice| service.answer(&answered_id, choice)),
        );
        if let Some(note) = note {
            let mut open = self.open();
            match open.prompts.get_mut(&signal_id) {
                Some(prompt) => prompt.note = Some(note),
                // Answered in the window while the notification was being shown.
                None => {
                    drop(open);
                    self.0.notifier.close(note);
                }
            }
        }
    }

    /// A choice on the prompt notification.
    fn answer(&self, signal_id: &str, choice: Choice) {
        let mut open = self.open();
        let Some(prompt) = open.prompts.get_mut(signal_id) else {
            // Already answered in the window or closed by a recording.
            return;
        };
        // The notification is gone whatever the choice was.
        prompt.note = None;
        let app = prompt.app;
        match choice {
            // The window prompt stays until it is answered there.
            Choice::Dismissed => return,
            Choice::Open => {
                drop(open);
                self.0.events.show_window();
                return;
            }
            Choice::Record | Choice::NotNow | Choice::Never | Choice::Stop => {}
        }
        open.prompts.remove(signal_id);
        drop(open);
        self.0.events.prompt_closed(signal_id);
        let result = match choice {
            Choice::Record => self.0.recorder.start_for(app),
            Choice::Never => self.set_rule(app, AppRule::Never),
            _ => Ok(()),
        };
        if let Err(err) = result {
            tracing::warn!(%err, "could not act on the consent prompt");
            self.show_error(&err);
        }
    }

    fn show_rule_note(&self, app: SourceApp) {
        let service = self.clone();
        let note = self.0.notifier.show(
            Note {
                summary: "Recording".into(),
                body: format!("Started by your rule for {}.", app.label()),
                buttons: vec![(Choice::Stop, "Stop".into())],
                sticky: true,
            },
            Box::new(move |choice| match choice {
                Choice::Stop => {
                    if let Err(err) = service.0.recorder.stop() {
                        tracing::warn!(%err, "could not stop the recording from the notification");
                        service.show_error(&err);
                    }
                }
                Choice::Open => service.0.events.show_window(),
                _ => {}
            }),
        );
        let old = std::mem::replace(&mut self.open().rule_note, note);
        if let Some(old) = old {
            self.0.notifier.close(old);
        }
    }

    fn show_error(&self, err: &AppError) {
        let service = self.clone();
        self.0.notifier.show(
            Note {
                summary: "Could not record".into(),
                body: err.to_string(),
                buttons: Vec::new(),
                sticky: false,
            },
            Box::new(move |choice| {
                if choice == Choice::Open {
                    service.0.events.show_window();
                }
            }),
        );
    }

    fn close_prompts(&self, matches: impl Fn(&Prompt) -> bool) {
        let mut closed = Vec::new();
        self.open().prompts.retain(|id, prompt| {
            let close = matches(prompt);
            if close {
                closed.push((id.clone(), prompt.note));
            }
            !close
        });
        for (signal_id, note) in closed {
            if let Some(note) = note {
                self.0.notifier.close(note);
            }
            self.0.events.prompt_closed(&signal_id);
        }
    }

    fn open(&self) -> MutexGuard<'_, Open> {
        // Only maps of open prompts live here, so a poisoned lock is still usable.
        self.0.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DetectorEvents for ConsentService {
    fn detected(&self, signal: &MeetingDetected) {
        self.on_detected(signal);
    }
}

/// Same words as the window prompt (`app/src/features/consent/apps.ts`).
fn prompt_note(app: SourceApp) -> Note {
    let (who, never) = match app {
        SourceApp::Browser => ("Your browser".to_owned(), "Never for browsers".to_owned()),
        _ => (app.label().to_owned(), format!("Never for {}", app.label())),
    };
    Note {
        summary: "Record this meeting?".into(),
        body: format!("{who} is using your microphone."),
        buttons: vec![
            (Choice::Record, "Record".into()),
            (Choice::NotNow, "Not now".into()),
            (Choice::Never, never),
        ],
        sticky: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeNotifier {
        shown: Mutex<Vec<(NoteId, Note, Option<OnChoice>)>>,
        closed: Mutex<Vec<NoteId>>,
        broken: bool,
    }

    impl Notifier for Arc<FakeNotifier> {
        fn show(&self, note: Note, on_choice: OnChoice) -> Option<NoteId> {
            if self.broken {
                return None;
            }
            let mut shown = self.shown.lock().unwrap();
            let id = NoteId(u32::try_from(shown.len()).unwrap() + 1);
            shown.push((id, note, Some(on_choice)));
            Some(id)
        }
        fn close(&self, id: NoteId) {
            self.closed.lock().unwrap().push(id);
        }
    }

    impl FakeNotifier {
        fn note(&self, n: usize) -> Note {
            self.shown.lock().unwrap()[n].1.clone()
        }
        fn count(&self) -> usize {
            self.shown.lock().unwrap().len()
        }
        /// Plays the user acting on note `n`.
        fn choose(&self, n: usize, choice: Choice) {
            let on_choice = self.shown.lock().unwrap()[n].2.take().unwrap();
            on_choice(choice);
        }
    }

    #[derive(Default)]
    struct FakeRecorder {
        starts: Mutex<Vec<SourceApp>>,
        stops: Mutex<u32>,
        fail: bool,
    }

    impl RecordingControl for FakeRecorder {
        fn start_for(&self, app: SourceApp) -> Result<(), AppError> {
            if self.fail {
                return Err(AppError::AudioDevice("No microphone found.".into()));
            }
            self.starts.lock().unwrap().push(app);
            Ok(())
        }
        fn stop(&self) -> Result<(), AppError> {
            *self.stops.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeEvents {
        prompts: Mutex<Vec<String>>,
        closed: Mutex<Vec<String>>,
        windows: Mutex<u32>,
    }

    impl ConsentEvents for FakeEvents {
        fn prompt(&self, signal: &MeetingDetected) {
            self.prompts.lock().unwrap().push(signal.signal_id.clone());
        }
        fn prompt_closed(&self, signal_id: &str) {
            self.closed.lock().unwrap().push(signal_id.into());
        }
        fn show_window(&self) {
            *self.windows.lock().unwrap() += 1;
        }
    }

    struct Harness {
        service: ConsentService,
        notifier: Arc<FakeNotifier>,
        recorder: Arc<FakeRecorder>,
        events: Arc<FakeEvents>,
    }

    impl Harness {
        fn new(recorder: FakeRecorder, notifier: FakeNotifier) -> Self {
            let notifier = Arc::new(notifier);
            let recorder = Arc::new(recorder);
            let events = Arc::new(FakeEvents::default());
            let service = ConsentService::new(
                Arc::new(Store::open_in_memory().unwrap()),
                recorder.clone(),
                Box::new(notifier.clone()),
                events.clone(),
            );
            Self {
                service,
                notifier,
                recorder,
                events,
            }
        }

        fn detect(&self, id: &str, app: SourceApp) {
            self.service.detected(&MeetingDetected {
                signal_id: id.into(),
                source_app: app,
                title: None,
                confidence: 0.9,
            });
        }

        fn closed(&self) -> Vec<String> {
            self.events.closed.lock().unwrap().clone()
        }
        fn prompts(&self) -> Vec<String> {
            self.events.prompts.lock().unwrap().clone()
        }
        fn starts(&self) -> Vec<SourceApp> {
            self.recorder.starts.lock().unwrap().clone()
        }
        fn rule(&self, app: SourceApp) -> AppRule {
            self.service
                .rules()
                .unwrap()
                .into_iter()
                .find(|entry| entry.source_app == app)
                .unwrap()
                .rule
        }
    }

    fn harness() -> Harness {
        Harness::new(FakeRecorder::default(), FakeNotifier::default())
    }

    #[test]
    fn fr_1_3_detection_prompts_and_record_starts_recording() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        assert_eq!(h.prompts(), ["s1"]);
        let note = h.notifier.note(0);
        assert_eq!(note.summary, "Record this meeting?");
        assert_eq!(note.body, "Zoom is using your microphone.");
        assert_eq!(
            note.buttons,
            [
                (Choice::Record, "Record".to_owned()),
                (Choice::NotNow, "Not now".to_owned()),
                (Choice::Never, "Never for Zoom".to_owned()),
            ]
        );
        assert!(note.sticky);
        assert!(h.starts().is_empty(), "nothing records before the answer");

        h.notifier.choose(0, Choice::Record);
        assert_eq!(h.starts(), [SourceApp::Zoom]);
        assert_eq!(h.closed(), ["s1"]);
    }

    #[test]
    fn fr_1_3_not_now_closes_the_prompt_without_recording() {
        let h = harness();
        h.detect("s1", SourceApp::Slack);
        h.notifier.choose(0, Choice::NotNow);
        assert!(h.starts().is_empty());
        assert_eq!(h.closed(), ["s1"]);
        assert_eq!(h.rule(SourceApp::Slack), AppRule::Ask);
    }

    #[test]
    fn fr_1_7_never_stores_the_rule_and_later_detections_are_ignored() {
        let h = harness();
        h.detect("s1", SourceApp::Browser);
        assert_eq!(h.notifier.note(0).buttons[2].1, "Never for browsers");
        assert_eq!(
            h.notifier.note(0).body,
            "Your browser is using your microphone."
        );
        h.notifier.choose(0, Choice::Never);
        assert_eq!(h.rule(SourceApp::Browser), AppRule::Never);
        assert_eq!(h.closed(), ["s1"]);

        h.detect("s2", SourceApp::Browser);
        assert_eq!(h.prompts(), ["s1"]);
        assert_eq!(h.notifier.count(), 1);
        assert!(h.starts().is_empty());
    }

    #[test]
    fn fr_1_7_always_records_at_once_and_its_note_can_stop() {
        let h = harness();
        h.service
            .set_rule(SourceApp::Teams, AppRule::Always)
            .unwrap();
        h.detect("s1", SourceApp::Teams);
        assert_eq!(h.starts(), [SourceApp::Teams]);
        assert!(h.prompts().is_empty(), "no prompt when a rule recorded");
        let note = h.notifier.note(0);
        assert_eq!(note.summary, "Recording");
        assert_eq!(note.body, "Started by your rule for Teams.");
        assert_eq!(note.buttons, [(Choice::Stop, "Stop".to_owned())]);

        h.notifier.choose(0, Choice::Stop);
        assert_eq!(*h.recorder.stops.lock().unwrap(), 1);
        h.service.recording_changed(Phase::Stopped);
        assert_eq!(*h.notifier.closed.lock().unwrap(), [NoteId(1)]);
    }

    #[test]
    fn always_rule_that_cannot_start_asks_instead() {
        let h = Harness::new(
            FakeRecorder {
                fail: true,
                ..FakeRecorder::default()
            },
            FakeNotifier::default(),
        );
        h.service
            .set_rule(SourceApp::Zoom, AppRule::Always)
            .unwrap();
        h.detect("s1", SourceApp::Zoom);
        assert_eq!(h.prompts(), ["s1"]);
        assert_eq!(h.notifier.note(0).summary, "Record this meeting?");
    }

    #[test]
    fn failed_record_from_the_notification_says_why() {
        let h = Harness::new(
            FakeRecorder {
                fail: true,
                ..FakeRecorder::default()
            },
            FakeNotifier::default(),
        );
        h.detect("s1", SourceApp::Zoom);
        h.notifier.choose(0, Choice::Record);
        let error = h.notifier.note(1);
        assert_eq!(error.summary, "Could not record");
        assert_eq!(error.body, "No microphone found.");
    }

    #[test]
    fn fr_1_3_a_recording_closes_every_open_prompt() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        h.detect("s2", SourceApp::Slack);
        h.service.recording_changed(Phase::Recording);
        let mut closed = h.closed();
        closed.sort();
        assert_eq!(closed, ["s1", "s2"]);
        assert_eq!(h.notifier.closed.lock().unwrap().len(), 2);
        // The closed notifications report back as dismissed; nothing else happens.
        h.notifier.choose(0, Choice::Dismissed);
        assert_eq!(h.closed().len(), 2);
    }

    #[test]
    fn never_from_the_window_closes_only_that_apps_prompt() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        h.detect("s2", SourceApp::Slack);
        h.service.set_rule(SourceApp::Zoom, AppRule::Never).unwrap();
        assert_eq!(h.closed(), ["s1"]);
        assert_eq!(*h.notifier.closed.lock().unwrap(), [NoteId(1)]);
        h.notifier.choose(0, Choice::Record);
        assert!(h.starts().is_empty(), "a closed prompt cannot record");
    }

    #[test]
    fn dismissed_notification_leaves_the_window_prompt() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        h.notifier.choose(0, Choice::Dismissed);
        assert!(h.closed().is_empty());
        // The recording later started from the window closes it.
        h.service.recording_changed(Phase::Recording);
        assert_eq!(h.closed(), ["s1"]);
        assert!(h.notifier.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn clicking_the_notification_opens_the_window() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        h.notifier.choose(0, Choice::Open);
        assert_eq!(*h.events.windows.lock().unwrap(), 1);
        assert!(h.closed().is_empty());
    }

    #[test]
    fn newer_detection_replaces_an_unanswered_prompt() {
        let h = harness();
        h.detect("s1", SourceApp::Zoom);
        h.detect("s2", SourceApp::Zoom);
        assert_eq!(h.closed(), ["s1"]);
        assert_eq!(h.prompts(), ["s1", "s2"]);
    }

    #[test]
    fn without_notifications_the_window_still_prompts() {
        let h = Harness::new(
            FakeRecorder::default(),
            FakeNotifier {
                broken: true,
                ..FakeNotifier::default()
            },
        );
        h.detect("s1", SourceApp::Discord);
        assert_eq!(h.prompts(), ["s1"]);
    }

    #[test]
    fn fr_1_7_lists_every_app_with_ask_by_default() {
        let h = harness();
        h.service
            .set_rule(SourceApp::Discord, AppRule::Always)
            .unwrap();
        let rules = h.service.rules().unwrap();
        assert_eq!(rules.len(), SourceApp::ALL.len());
        assert_eq!(h.rule(SourceApp::Discord), AppRule::Always);
        assert_eq!(h.rule(SourceApp::Zoom), AppRule::Ask);
        let json = serde_json::to_value(&rules[0]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "sourceApp": "zoom", "rule": "ask" })
        );
    }

    #[test]
    fn notification_keys_round_trip() {
        for choice in [
            Choice::Record,
            Choice::NotNow,
            Choice::Never,
            Choice::Stop,
            Choice::Open,
        ] {
            assert_eq!(Choice::from_key(choice.key()), choice);
        }
        assert_eq!(Choice::from_key("something"), Choice::Dismissed);
    }
}
