//! `jobs` table: the persistent post-call queue (NFR-7, ADR-019). One row per meeting step.

use rusqlite::{params, Connection, OptionalExtension};

use super::meetings::MeetingStatus;
use super::{now_ms, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// A queued job as the worker needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub meeting_id: String,
    pub step: String,
    /// Attempts already made.
    pub attempts: u32,
}

pub struct JobRepo<'a> {
    conn: &'a Connection,
}

impl<'a> JobRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Queues `step` for `meeting_id`, due at `run_at` (epoch ms).
    pub fn enqueue(&self, meeting_id: &str, step: &str, run_at: i64) -> Result<String, StoreError> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO jobs (id, meeting_id, step, status, attempts, next_run_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?6)",
            params![id, meeting_id, step, JobStatus::Queued.as_str(), run_at, now],
        )?;
        Ok(id)
    }

    /// The queued job due first, if one is due at `now`.
    pub fn next_due(&self, now: i64) -> Result<Option<Job>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, meeting_id, step, attempts FROM jobs
                 WHERE status = ?1 AND next_run_at <= ?2
                 ORDER BY next_run_at, created_at, id LIMIT 1",
                params![JobStatus::Queued.as_str(), now],
                |row| {
                    Ok(Job {
                        id: row.get(0)?,
                        meeting_id: row.get(1)?,
                        step: row.get(2)?,
                        attempts: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// When the next queued job is due.
    pub fn next_run_at(&self) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row(
            "SELECT min(next_run_at) FROM jobs WHERE status = ?1",
            [JobStatus::Queued.as_str()],
            |row| row.get(0),
        )?)
    }

    /// Marks the job running and returns the attempt number (from 1).
    pub fn start(&self, id: &str) -> Result<u32, StoreError> {
        Ok(self.conn.query_row(
            "UPDATE jobs SET status = ?2, attempts = attempts + 1, updated_at = ?3
             WHERE id = ?1 RETURNING attempts",
            params![id, JobStatus::Running.as_str(), now_ms()],
            |row| row.get(0),
        )?)
    }

    pub fn done(&self, id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE jobs SET status = ?2, error = NULL, next_run_at = NULL, updated_at = ?3 WHERE id = ?1",
            params![id, JobStatus::Done.as_str(), now_ms()],
        )?;
        Ok(())
    }

    /// Queues the job again at `run_at`, keeping why the last attempt failed.
    pub fn retry(&self, id: &str, run_at: i64, error: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE jobs SET status = ?2, next_run_at = ?3, error = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, JobStatus::Queued.as_str(), run_at, error, now_ms()],
        )?;
        Ok(())
    }

    pub fn fail(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE jobs SET status = ?2, next_run_at = NULL, error = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, JobStatus::Failed.as_str(), error, now_ms()],
        )?;
        Ok(())
    }

    /// Parks the job until something it needs exists (an API key): queued with no run
    /// time, and the attempt it just made does not count.
    pub fn park(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE jobs SET status = ?2, next_run_at = NULL, attempts = max(attempts - 1, 0),
             error = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, JobStatus::Queued.as_str(), error, now_ms()],
        )?;
        Ok(())
    }

    /// Makes parked jobs due at `now`. Returns how many.
    pub fn release_parked(&self, now: i64) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "UPDATE jobs SET next_run_at = ?2, updated_at = ?2 WHERE status = ?1 AND next_run_at IS NULL",
            params![JobStatus::Queued.as_str(), now],
        )?)
    }

    /// Jobs a crash or quit left `running` go back to the queue, due now. Steps are
    /// idempotent, so running one again is safe. Returns how many.
    pub fn requeue_running(&self, now: i64) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "UPDATE jobs SET status = ?1, next_run_at = ?3, updated_at = ?3 WHERE status = ?2",
            params![JobStatus::Queued.as_str(), JobStatus::Running.as_str(), now],
        )?)
    }

    /// Meetings waiting for processing that have no jobs yet, oldest first.
    pub fn meetings_without_jobs(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM meetings m WHERE status = ?1
             AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.meeting_id = m.id)
             ORDER BY started_at",
        )?;
        let rows = stmt.query_map([MeetingStatus::Processing.as_str()], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    #[cfg(test)]
    pub fn get(&self, id: &str) -> Result<(String, u32, Option<i64>, Option<String>), StoreError> {
        Ok(self.conn.query_row(
            "SELECT status, attempts, next_run_at, error FROM jobs WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meetings::MeetingRepo;
    use crate::store::Store;

    fn processing(conn: &Connection, started_at: i64) -> String {
        let repo = MeetingRepo::new(conn);
        let meeting = repo
            .create_recording("Standup", "zoom", started_at)
            .unwrap();
        repo.finish(
            &meeting.id,
            started_at + 60_000,
            60,
            MeetingStatus::Processing,
        )
        .unwrap();
        meeting.id
    }

    #[test]
    fn nfr_7_queue_order_attempts_retry_and_failure() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = JobRepo::new(&conn);
        let m = processing(&conn, 1_000);

        let late = repo.enqueue(&m, "merge", 5_000).unwrap();
        let early = repo.enqueue(&m, "transcribe_mic", 2_000).unwrap();
        assert_eq!(repo.next_due(1_999).unwrap(), None);
        assert_eq!(repo.next_run_at().unwrap(), Some(2_000));
        let job = repo.next_due(10_000).unwrap().unwrap();
        assert_eq!((job.id.as_str(), job.attempts), (early.as_str(), 0));

        assert_eq!(repo.start(&early).unwrap(), 1);
        repo.retry(&early, 9_000, "offline").unwrap();
        assert_eq!(
            repo.get(&early).unwrap(),
            ("queued".into(), 1, Some(9_000), Some("offline".into()))
        );
        assert_eq!(repo.next_due(8_000).unwrap().unwrap().id, late);
        assert_eq!(repo.start(&early).unwrap(), 2);
        repo.done(&early).unwrap();
        assert_eq!(repo.get(&early).unwrap(), ("done".into(), 2, None, None));

        repo.start(&late).unwrap();
        repo.fail(&late, "rejected").unwrap();
        assert_eq!(repo.get(&late).unwrap().0, "failed");
        assert_eq!(repo.next_due(i64::MAX).unwrap(), None);
        assert_eq!(repo.next_run_at().unwrap(), None);
    }

    #[test]
    fn nfr_7_running_jobs_are_requeued_after_a_crash() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = JobRepo::new(&conn);
        let m = processing(&conn, 1_000);
        let id = repo.enqueue(&m, "transcribe_mic", 0).unwrap();
        repo.start(&id).unwrap();
        assert_eq!(repo.requeue_running(7_000).unwrap(), 1);
        assert_eq!(
            repo.get(&id).unwrap(),
            ("queued".into(), 1, Some(7_000), None)
        );
    }

    #[test]
    fn nfr_7_parked_jobs_wait_until_released_without_using_attempts() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = JobRepo::new(&conn);
        let m = processing(&conn, 1_000);
        let id = repo.enqueue(&m, "transcribe_mic", 0).unwrap();
        repo.start(&id).unwrap();
        repo.park(&id, "No key").unwrap();
        assert_eq!(
            repo.get(&id).unwrap(),
            ("queued".into(), 0, None, Some("No key".into()))
        );
        assert_eq!(repo.next_due(i64::MAX).unwrap(), None);
        assert_eq!(repo.next_run_at().unwrap(), None);
        assert_eq!(repo.release_parked(9_000).unwrap(), 1);
        assert_eq!(repo.next_due(9_000).unwrap().unwrap().id, id);
        assert_eq!(repo.release_parked(10_000).unwrap(), 0);
    }

    #[test]
    fn only_processing_meetings_without_jobs_need_jobs() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = JobRepo::new(&conn);
        let older = processing(&conn, 1_000);
        let newer = processing(&conn, 2_000);
        let meetings = MeetingRepo::new(&conn);
        let cut = meetings.create_recording("Cut", "zoom", 500).unwrap();
        meetings
            .finish(&cut.id, 1_500, 1, MeetingStatus::Interrupted)
            .unwrap();
        meetings.create_recording("Live", "zoom", 600).unwrap();

        assert_eq!(
            repo.meetings_without_jobs().unwrap(),
            [older.clone(), newer.clone()]
        );
        repo.enqueue(&older, "transcribe_mic", 0).unwrap();
        assert_eq!(repo.meetings_without_jobs().unwrap(), [newer]);
    }
}
