//! `reports`, `action_items` and `scores` (FR-4.1, FR-4.2, FR-5.1 to FR-5.5, ADR-024).
//! The metrics step owns the `metric:*` rows it computes; the analyze step owns the report,
//! the action items, the headline scores and the metrics that come from the analysis. Each
//! writes its rows whole, in one transaction, so a rerun replaces them.

use rusqlite::{params, Connection};
use serde_json::Value;

use super::{now_ms, StoreError};

/// Metric rows written by the analyze step, not the metrics step (docs/07).
pub const ANALYSIS_METRICS: [&str; 2] = ["metric:action_items_count", "metric:decisions_count"];
/// Headline score kinds (FR-5.1 to FR-5.4).
pub const HEADLINE_KINDS: [&str; 4] = ["engagement", "value", "my_performance", "productivity"];

/// JSON columns hold the analysis as validated in Rust (docs/05).
#[derive(Debug, Clone, PartialEq)]
pub struct NewReport {
    pub summary_md: String,
    pub key_points: Value,
    pub decisions: Value,
    pub open_questions: Value,
    pub suggestions: Value,
    pub chapters: Value,
    pub model_used: String,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActionItem {
    pub owner_label: Option<String>,
    pub task: String,
    /// YYYY-MM-DD.
    pub due_date: Option<String>,
    pub segment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewScore {
    /// `engagement` ... or `metric:<name>`.
    pub kind: String,
    pub value: f64,
    /// `{ metrics, segment_ids, rationale }` for headline scores; `{}` for metrics.
    pub evidence: Value,
}

pub struct ReportRepo<'a> {
    conn: &'a Connection,
}

impl<'a> ReportRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replaces the computed `metric:*` rows, leaving the analysis ones alone.
    pub fn replace_metrics(
        &self,
        meeting_id: &str,
        metrics: &[(String, f64)],
    ) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM scores WHERE meeting_id = ?1 AND kind LIKE 'metric:%'
             AND kind NOT IN (?2, ?3)",
            params![meeting_id, ANALYSIS_METRICS[0], ANALYSIS_METRICS[1]],
        )?;
        let now = now_ms();
        for (name, value) in metrics {
            insert_score(
                &tx,
                meeting_id,
                &format!("metric:{name}"),
                *value,
                "{}",
                now,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Replaces the report, the action items, the headline scores and the analysis metrics
    /// in one transaction (rule 6: never half saved). Headline kinds missing from `scores`
    /// are removed, so a meeting with too little data keeps none.
    pub fn save_analysis(
        &self,
        meeting_id: &str,
        report: &NewReport,
        actions: &[NewActionItem],
        scores: &[NewScore],
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM reports WHERE meeting_id = ?1", [meeting_id])?;
        tx.execute(
            "INSERT INTO reports (id, meeting_id, summary_md, key_points, decisions, open_questions,
             suggestions, chapters, model_used, cost_usd, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                uuid::Uuid::now_v7().to_string(),
                meeting_id,
                report.summary_md,
                report.key_points.to_string(),
                report.decisions.to_string(),
                report.open_questions.to_string(),
                report.suggestions.to_string(),
                report.chapters.to_string(),
                report.model_used,
                report.cost_usd,
                now
            ],
        )?;
        tx.execute(
            "DELETE FROM action_items WHERE meeting_id = ?1",
            [meeting_id],
        )?;
        for item in actions {
            tx.execute(
                "INSERT INTO action_items (id, meeting_id, owner_label, task, due_date, segment_id,
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    meeting_id,
                    item.owner_label,
                    item.task,
                    item.due_date,
                    item.segment_id,
                    now
                ],
            )?;
        }
        for kind in HEADLINE_KINDS.iter().chain(&ANALYSIS_METRICS) {
            tx.execute(
                "DELETE FROM scores WHERE meeting_id = ?1 AND kind = ?2",
                params![meeting_id, kind],
            )?;
        }
        for score in scores {
            insert_score(
                &tx,
                meeting_id,
                &score.kind,
                score.value,
                &score.evidence.to_string(),
                now,
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn insert_score(
    conn: &Connection,
    meeting_id: &str,
    kind: &str,
    value: f64,
    evidence: &str,
    now: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO scores (id, meeting_id, kind, value, evidence, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            uuid::Uuid::now_v7().to_string(),
            meeting_id,
            kind,
            value,
            evidence,
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::meetings::MeetingRepo;
    use crate::store::Store;

    fn kinds(conn: &Connection, meeting_id: &str) -> Vec<(String, f64)> {
        let mut stmt = conn
            .prepare("SELECT kind, value FROM scores WHERE meeting_id = ?1 ORDER BY kind")
            .unwrap();
        stmt.query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn report(summary: &str) -> NewReport {
        NewReport {
            summary_md: summary.into(),
            key_points: json!(["a"]),
            decisions: json!([]),
            open_questions: json!([]),
            suggestions: json!([]),
            chapters: json!([]),
            model_used: "model".into(),
            cost_usd: Some(0.01),
        }
    }

    fn score(kind: &str, value: f64) -> NewScore {
        NewScore {
            kind: kind.into(),
            value,
            evidence: json!({}),
        }
    }

    #[test]
    fn metrics_and_analysis_rows_replace_only_their_own() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let id = MeetingRepo::new(&conn)
            .create_recording("Standup", "zoom", 0)
            .unwrap()
            .id;
        let repo = ReportRepo::new(&conn);
        repo.replace_metrics(&id, &[("balance".into(), 0.5), ("my_wpm".into(), 140.0)])
            .unwrap();
        let item = NewActionItem {
            owner_label: Some("Me".into()),
            task: "Write notes".into(),
            due_date: None,
            segment_id: None,
        };
        repo.save_analysis(
            &id,
            &report("first"),
            &[item.clone(), item.clone()],
            &[score("value", 70.0), score("metric:decisions_count", 1.0)],
        )
        .unwrap();

        // A metrics rerun keeps the analysis rows; one metric is no longer defined.
        repo.replace_metrics(&id, &[("balance".into(), 0.75)])
            .unwrap();
        assert_eq!(
            kinds(&conn, &id),
            [
                ("metric:balance".to_owned(), 0.75),
                ("metric:decisions_count".to_owned(), 1.0),
                ("value".to_owned(), 70.0),
            ]
        );

        // An analysis rerun replaces the report and items; a headline it lacks is removed.
        repo.save_analysis(
            &id,
            &report("second"),
            &[item],
            &[score("engagement", 40.0)],
        )
        .unwrap();
        assert_eq!(
            kinds(&conn, &id),
            [
                ("engagement".to_owned(), 40.0),
                ("metric:balance".to_owned(), 0.75)
            ]
        );
        let (reports, summary): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(summary_md) FROM reports WHERE meeting_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((reports, summary.as_str()), (1, "second"));
        let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM action_items WHERE meeting_id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(items, 1);
    }
}
