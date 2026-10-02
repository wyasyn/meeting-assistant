//! Persistent job queue and pipeline steps (NFR-7, ADR-019). Each post-call step of a
//! meeting is a row in `jobs`, run in pipeline order on one worker thread; finishing a step
//! queues the next. Retryable failures come back with backoff, others fail the meeting.
//! Audio is never touched here, so a failed meeting can be processed again (FR-4.6).

pub mod steps;

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::error::AppError;
use crate::store::jobs::{JobRepo, JobStatus};
use crate::store::meetings::{MeetingRepo, MeetingStatus};
use crate::store::{now_ms, Store};

/// Attempts per step before the meeting is marked failed.
pub const MAX_ATTEMPTS: u32 = 6;
const BACKOFF_FIRST: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);
/// The worker looks at the queue at least this often, even without a wake-up.
const IDLE_POLL: Duration = Duration::from_secs(30);

/// Post-call steps, in pipeline order (docs/04 "Pipeline steps").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "metrics to notify get runners in Phase 2")]
pub enum Step {
    TranscribeMic,
    TranscribeSystem,
    Merge,
    Metrics,
    Analyze,
    Index,
    Notify,
}

impl Step {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TranscribeMic => "transcribe_mic",
            Self::TranscribeSystem => "transcribe_system",
            Self::Merge => "merge",
            Self::Metrics => "metrics",
            Self::Analyze => "analyze",
            Self::Index => "index",
            Self::Notify => "notify",
        }
    }
}

/// One step's work for one meeting. Must be idempotent: a crash or a retry runs it again.
/// Runs on the worker thread, so blocking is fine; errors that may pass with time (offline,
/// rate limits) must be `retryable`.
pub trait StepRunner: Send + Sync {
    fn run(&self, meeting_id: &str) -> Result<(), AppError>;
}

/// The steps a meeting goes through, in order.
#[derive(Default)]
pub struct Pipeline {
    steps: Vec<(Step, Box<dyn StepRunner>)>,
}

impl Pipeline {
    pub fn with(mut self, step: Step, runner: Box<dyn StepRunner>) -> Self {
        self.steps.push((step, runner));
        self
    }

    fn first(&self) -> Option<Step> {
        self.steps.first().map(|(step, _)| *step)
    }

    fn position(&self, step: &str) -> Option<usize> {
        self.steps.iter().position(|(s, _)| s.as_str() == step)
    }

    fn after(&self, index: usize) -> Option<Step> {
        self.steps.get(index + 1).map(|(step, _)| *step)
    }
}

/// Wait before attempt `attempt + 1`: 30 s, 2 min, 8 min, 32 min, then 1 h.
pub fn backoff(attempt: u32) -> Duration {
    let factor = 4u32.saturating_pow(attempt.saturating_sub(1));
    BACKOFF_FIRST.saturating_mul(factor).min(BACKOFF_MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressStatus {
    /// Waiting for a retry; `error` says why the last attempt failed.
    Queued,
    Running,
    Done,
    Failed,
}

impl From<JobStatus> for ProgressStatus {
    fn from(status: JobStatus) -> Self {
        match status {
            JobStatus::Queued => Self::Queued,
            JobStatus::Running => Self::Running,
            JobStatus::Done => Self::Done,
            JobStatus::Failed => Self::Failed,
        }
    }
}

/// Payload of `job:progress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub meeting_id: String,
    pub step: String,
    pub status: ProgressStatus,
    pub attempt: u32,
    /// Readable, never a path or transcript text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where progress goes: the `job:progress` event in the app, a fake in tests.
pub trait JobEvents: Send + Sync {
    fn progress(&self, progress: &JobProgress);
}

/// The queue logic, driven by the worker thread or directly by tests with their own clock.
struct Queue {
    store: Arc<Store>,
    pipeline: Pipeline,
    events: Arc<dyn JobEvents>,
}

impl Queue {
    /// Queues the first step for meetings that are waiting without jobs, for example
    /// because the app quit between stopping and queueing.
    fn adopt(&self, now: i64) -> Result<(), AppError> {
        let Some(first) = self.pipeline.first() else {
            return Ok(());
        };
        let conn = self.store.conn()?;
        let repo = JobRepo::new(&conn);
        for meeting_id in repo.meetings_without_jobs()? {
            repo.enqueue(&meeting_id, first.as_str(), now)?;
            tracing::info!("meeting queued for processing");
        }
        Ok(())
    }

    /// Runs the job due first, if any. Returns whether one ran.
    fn run_due(&self, now: i64) -> Result<bool, AppError> {
        self.adopt(now)?;
        let (job, attempt) = {
            let conn = self.store.conn()?;
            let repo = JobRepo::new(&conn);
            let Some(job) = repo.next_due(now)? else {
                return Ok(false);
            };
            let attempt = repo.start(&job.id)?;
            (job, attempt)
        };
        let progress = |status: JobStatus, error: Option<String>| {
            self.events.progress(&JobProgress {
                meeting_id: job.meeting_id.clone(),
                step: job.step.clone(),
                status: status.into(),
                attempt,
                error,
            });
        };
        progress(JobStatus::Running, None);
        tracing::info!(step = job.step, attempt, "job started");

        // The connection is free while the step runs; steps use the store themselves.
        let index = self.pipeline.position(&job.step);
        let result = match index {
            Some(i) => self.pipeline.steps[i].1.run(&job.meeting_id),
            None => Err(AppError::internal(format!(
                "no runner for step {}",
                job.step
            ))),
        };

        let conn = self.store.conn()?;
        let repo = JobRepo::new(&conn);
        match result {
            Ok(()) => {
                repo.done(&job.id)?;
                tracing::info!(step = job.step, "job done");
                match index.and_then(|i| self.pipeline.after(i)) {
                    Some(next) => {
                        repo.enqueue(&job.meeting_id, next.as_str(), now)?;
                    }
                    None => {
                        MeetingRepo::new(&conn)
                            .set_status(&job.meeting_id, MeetingStatus::Ready)?;
                        tracing::info!("meeting processed");
                    }
                }
                drop(conn);
                progress(JobStatus::Done, None);
            }
            // Waits for the user to add a key, however long that takes (ADR-021).
            Err(err @ AppError::NoApiKey(_)) => {
                let message = err.to_string();
                repo.park(&job.id, &message)?;
                tracing::info!(step = job.step, "job waits for an api key");
                drop(conn);
                progress(JobStatus::Queued, Some(message));
            }
            Err(err) if err.retryable() && attempt < MAX_ATTEMPTS => {
                let message = err.to_string();
                let wait = i64::try_from(backoff(attempt).as_millis()).unwrap_or(i64::MAX);
                repo.retry(&job.id, now.saturating_add(wait), &message)?;
                tracing::warn!(
                    step = job.step,
                    attempt,
                    code = err.code(),
                    "job failed, retrying"
                );
                drop(conn);
                progress(JobStatus::Queued, Some(message));
            }
            Err(err) => {
                let message = err.to_string();
                repo.fail(&job.id, &message)?;
                MeetingRepo::new(&conn).set_status(&job.meeting_id, MeetingStatus::Failed)?;
                tracing::warn!(step = job.step, attempt, code = err.code(), "job failed");
                drop(conn);
                progress(JobStatus::Failed, Some(message));
            }
        }
        Ok(true)
    }

    /// Runs parked jobs again, for example after an API key was saved.
    fn release_parked(&self, now: i64) {
        let released = self
            .store
            .conn()
            .map_err(AppError::from)
            .and_then(|conn| Ok(JobRepo::new(&conn).release_parked(now)?));
        match released {
            Ok(0) => {}
            Ok(n) => tracing::info!(jobs = n, "parked jobs released"),
            Err(err) => tracing::warn!(%err, "could not release parked jobs"),
        }
    }

    /// How long the worker may sleep.
    fn wait(&self, now: i64) -> Duration {
        let next = self
            .store
            .conn()
            .map_err(AppError::from)
            .and_then(|conn| Ok(JobRepo::new(&conn).next_run_at()?));
        match next {
            Ok(Some(at)) => {
                Duration::from_millis(u64::try_from(at - now).unwrap_or(0)).min(IDLE_POLL)
            }
            _ => IDLE_POLL,
        }
    }
}

enum Signal {
    Wake,
    /// Something parked jobs wait for may exist now.
    Resume,
}

/// Runs the queue on its own thread. Managed as Tauri state.
pub struct JobService {
    signal: Mutex<Option<Sender<Signal>>>,
}

impl JobService {
    /// Requeues jobs a crash left running, then starts the worker.
    pub fn start(store: Arc<Store>, pipeline: Pipeline, events: Arc<dyn JobEvents>) -> Self {
        match store
            .conn()
            .map_err(AppError::from)
            .and_then(|conn| Ok(JobRepo::new(&conn).requeue_running(now_ms())?))
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(jobs = n, "resumed interrupted jobs"),
            Err(err) => tracing::warn!(%err, "could not resume interrupted jobs"),
        }
        let queue = Queue {
            store,
            pipeline,
            events,
        };
        // A key may have been saved while jobs were parked; if not, they park again.
        queue.release_parked(now_ms());
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("jobs".into())
            .spawn(move || loop {
                let ran = queue.run_due(now_ms()).unwrap_or_else(|err| {
                    tracing::warn!(%err, "job queue error");
                    false
                });
                let wait = if ran {
                    Duration::ZERO
                } else {
                    queue.wait(now_ms())
                };
                match rx.recv_timeout(wait) {
                    Ok(Signal::Resume) => queue.release_parked(now_ms()),
                    Ok(Signal::Wake) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            });
        let signal = match spawned {
            Ok(_) => Some(tx),
            Err(err) => {
                tracing::warn!(%err, "could not start the job worker");
                None
            }
        };
        Self {
            signal: Mutex::new(signal),
        }
    }

    /// Looks at the queue now, for example after a recording was saved.
    pub fn wake(&self) {
        self.send(Signal::Wake);
    }

    /// Runs jobs parked for a missing API key again, after one was saved.
    pub fn resume(&self) {
        self.send(Signal::Resume);
    }

    fn send(&self, signal: Signal) {
        if let Ok(tx) = self.signal.lock() {
            if let Some(tx) = tx.as_ref() {
                let _ = tx.send(signal);
            }
        }
    }

    /// Stops the worker after its current step without waiting for it: a long step must
    /// not hold up quitting, and the job is requeued at the next start.
    pub fn shutdown(&self) {
        if let Ok(mut signal) = self.signal.lock() {
            signal.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::jobs::Job;

    /// Fails with the scripted errors first, then succeeds.
    struct FakeStep {
        name: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        errors: Mutex<Vec<AppError>>,
    }

    impl StepRunner for FakeStep {
        fn run(&self, _meeting_id: &str) -> Result<(), AppError> {
            self.log.lock().unwrap().push(self.name);
            let mut errors = self.errors.lock().unwrap();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.remove(0))
            }
        }
    }

    #[derive(Default)]
    struct FakeEvents(Mutex<Vec<JobProgress>>);

    impl JobEvents for FakeEvents {
        fn progress(&self, progress: &JobProgress) {
            self.0.lock().unwrap().push(progress.clone());
        }
    }

    impl FakeEvents {
        fn statuses(&self) -> Vec<(String, ProgressStatus, u32)> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|p| (p.step.clone(), p.status, p.attempt))
                .collect()
        }
    }

    type Log = Arc<Mutex<Vec<&'static str>>>;

    fn step(log: &Log, name: &'static str, errors: Vec<AppError>) -> Box<dyn StepRunner> {
        Box::new(FakeStep {
            name,
            log: log.clone(),
            errors: Mutex::new(errors),
        })
    }

    fn offline() -> AppError {
        AppError::ProviderUnavailable("The provider could not be reached.".into())
    }

    struct Harness {
        queue: Queue,
        events: Arc<FakeEvents>,
        log: Log,
        meeting_id: String,
    }

    impl Harness {
        fn new(pipeline: impl FnOnce(&Log) -> Pipeline) -> Self {
            let store = Arc::new(Store::open_in_memory().unwrap());
            let meeting_id = {
                let conn = store.conn().unwrap();
                let repo = MeetingRepo::new(&conn);
                let m = repo.create_recording("Standup", "zoom", 1_000).unwrap();
                repo.finish(&m.id, 61_000, 60, MeetingStatus::Processing)
                    .unwrap();
                m.id
            };
            let log = Log::default();
            let events = Arc::new(FakeEvents::default());
            let queue = Queue {
                store,
                pipeline: pipeline(&log),
                events: events.clone(),
            };
            Self {
                queue,
                events,
                log,
                meeting_id,
            }
        }

        fn status(&self) -> String {
            MeetingRepo::new(&self.queue.store.conn().unwrap())
                .status(&self.meeting_id)
                .unwrap()
                .unwrap()
        }

        /// Runs every job due at `now`.
        fn drain(&self, now: i64) -> usize {
            let mut ran = 0;
            while self.queue.run_due(now).unwrap() {
                ran += 1;
            }
            ran
        }
    }

    #[test]
    fn nfr_7_steps_run_in_order_and_the_meeting_ends_ready() {
        let h = Harness::new(|log| {
            Pipeline::default()
                .with(Step::TranscribeMic, step(log, "mic", vec![]))
                .with(Step::TranscribeSystem, step(log, "sys", vec![]))
                .with(Step::Merge, step(log, "merge", vec![]))
        });
        assert_eq!(h.drain(0), 3);
        assert_eq!(*h.log.lock().unwrap(), ["mic", "sys", "merge"]);
        assert_eq!(h.status(), "ready");
        use ProgressStatus::{Done, Running};
        assert_eq!(
            h.events.statuses(),
            [
                ("transcribe_mic".into(), Running, 1),
                ("transcribe_mic".into(), Done, 1),
                ("transcribe_system".into(), Running, 1),
                ("transcribe_system".into(), Done, 1),
                ("merge".into(), Running, 1),
                ("merge".into(), Done, 1),
            ]
        );
        assert!(h
            .events
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|p| p.meeting_id == h.meeting_id));
        assert_eq!(h.drain(i64::MAX), 0, "nothing runs twice");
    }

    #[test]
    fn nfr_7_retryable_failure_waits_for_the_backoff() {
        let h = Harness::new(|log| {
            Pipeline::default().with(
                Step::TranscribeMic,
                step(log, "mic", vec![offline(), offline()]),
            )
        });
        assert_eq!(h.drain(0), 1);
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.status, ProgressStatus::Queued);
        assert_eq!(
            last.error.as_deref(),
            Some("The provider could not be reached.")
        );
        assert_eq!(h.status(), "processing");

        assert_eq!(h.drain(29_999), 0, "not before the backoff");
        assert_eq!(h.queue.wait(29_000), Duration::from_secs(1));
        assert_eq!(h.drain(30_000), 1);
        // Second wait is 2 minutes from the second attempt.
        assert_eq!(h.drain(30_000 + 119_999), 0);
        assert_eq!(h.drain(30_000 + 120_000), 1);
        assert_eq!(h.status(), "ready");
        assert_eq!(h.log.lock().unwrap().len(), 3);
    }

    #[test]
    fn nfr_7_gives_up_after_max_attempts_and_keeps_the_meeting() {
        let h = Harness::new(|log| {
            Pipeline::default().with(
                Step::TranscribeMic,
                step(log, "mic", (0..MAX_ATTEMPTS).map(|_| offline()).collect()),
            )
        });
        let mut now = 0;
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(h.drain(now), 1);
            now += i64::try_from(BACKOFF_MAX.as_millis()).unwrap();
        }
        assert_eq!(h.status(), "failed");
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            (last.status, last.attempt),
            (ProgressStatus::Failed, MAX_ATTEMPTS)
        );
        assert_eq!(h.drain(i64::MAX), 0);
    }

    #[test]
    fn nfr_7_non_retryable_failure_fails_at_once() {
        let h = Harness::new(|log| {
            Pipeline::default()
                .with(
                    Step::TranscribeMic,
                    step(
                        log,
                        "mic",
                        vec![AppError::NoApiKey("Add an API key in settings.".into())],
                    ),
                )
                .with(Step::Merge, step(log, "merge", vec![]))
        });
        assert_eq!(h.drain(i64::MAX), 1);
        assert_eq!(h.status(), "failed");
        assert_eq!(*h.log.lock().unwrap(), ["mic"]);
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.error.as_deref(), Some("Add an API key in settings."));
    }

    #[test]
    fn nfr_7_a_missing_key_parks_the_job_until_resumed() {
        let no_key = || AppError::NoApiKey("No Gemini API key is saved.".into());
        let h = Harness::new(|log| {
            Pipeline::default().with(
                Step::TranscribeMic,
                step(log, "mic", vec![no_key(), no_key()]),
            )
        });
        assert_eq!(h.drain(i64::MAX), 1);
        assert_eq!(h.status(), "processing");
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            (last.status, last.error.as_deref()),
            (ProgressStatus::Queued, Some("No Gemini API key is saved."))
        );
        assert_eq!(h.drain(i64::MAX), 0, "parked, not retried");
        assert_eq!(h.queue.wait(0), IDLE_POLL);

        // Still no key: it parks again and the attempt does not count.
        h.queue.release_parked(10);
        assert_eq!(h.drain(10), 1);
        h.queue.release_parked(20);
        assert_eq!(h.drain(20), 1);
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.status, last.attempt), (ProgressStatus::Done, 1));
        assert_eq!(h.status(), "ready");
    }

    #[test]
    fn nfr_7_a_job_cut_off_by_a_crash_runs_again() {
        let h = Harness::new(|log| {
            Pipeline::default().with(Step::TranscribeMic, step(log, "mic", vec![]))
        });
        // The app died mid-step: the row is running, attempt 1.
        h.queue.adopt(0).unwrap();
        let with_repo = |f: &dyn Fn(&JobRepo)| f(&JobRepo::new(&h.queue.store.conn().unwrap()));
        with_repo(&|repo| {
            let Job { id, .. } = repo.next_due(0).unwrap().unwrap();
            repo.start(&id).unwrap();
        });
        assert_eq!(h.drain(i64::MAX), 0, "running jobs are not picked up");
        with_repo(&|repo| assert_eq!(repo.requeue_running(5).unwrap(), 1));
        assert_eq!(h.drain(5), 1);
        let last = h.events.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.status, last.attempt), (ProgressStatus::Done, 2));
        assert_eq!(h.status(), "ready");
    }

    #[test]
    fn empty_pipeline_leaves_meetings_waiting() {
        let h = Harness::new(|_| Pipeline::default());
        assert_eq!(h.drain(i64::MAX), 0);
        assert_eq!(h.status(), "processing");
        assert_eq!(h.queue.wait(0), IDLE_POLL);
    }

    #[test]
    fn backoff_grows_by_four_up_to_an_hour() {
        let mins = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(backoff(1), Duration::from_secs(30));
        assert_eq!(backoff(2), mins(2));
        assert_eq!(backoff(3), mins(8));
        assert_eq!(backoff(4), mins(32));
        assert_eq!(backoff(5), mins(60));
        assert_eq!(backoff(40), mins(60));
    }

    #[test]
    fn progress_uses_the_contract_shape() {
        let mut progress = JobProgress {
            meeting_id: "m1".into(),
            step: "merge".into(),
            status: ProgressStatus::Running,
            attempt: 1,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            serde_json::json!({ "meetingId": "m1", "step": "merge", "status": "running", "attempt": 1 })
        );
        progress.error = Some("offline".into());
        assert_eq!(serde_json::to_value(&progress).unwrap()["error"], "offline");
    }

    #[test]
    fn service_runs_queued_work_on_its_thread() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        {
            let conn = store.conn().unwrap();
            let repo = MeetingRepo::new(&conn);
            let m = repo.create_recording("Standup", "zoom", 1_000).unwrap();
            repo.finish(&m.id, 61_000, 60, MeetingStatus::Processing)
                .unwrap();
        }
        let log = Log::default();
        let events = Arc::new(FakeEvents::default());
        let service = JobService::start(
            store,
            Pipeline::default().with(Step::TranscribeMic, step(&log, "mic", vec![])),
            events.clone(),
        );
        service.wake();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while events.0.lock().unwrap().len() < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not run the job"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        service.shutdown();
        assert_eq!(*log.lock().unwrap(), ["mic"]);
    }
}
