//! Pipeline steps 2.2 and 2.3 (ADR-024, ADR-025): store the deterministic metrics, then ask
//! the LLM for the report and save it with the action items and the headline scores
//! (FR-4.1, FR-4.2, FR-5.1 to FR-5.4, rule 5, rule 6).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::StepRunner;
use crate::error::AppError;
use crate::metrics::{self, scoring, Metrics, Timing};
use crate::providers::analysis::{AnalysisJson, Judged};
use crate::providers::{AnalyzeRequest, ProviderId, ProviderService};
use crate::store::meetings::{AnalysisInfo, MeetingRepo};
use crate::store::reports::{NewActionItem, NewReport, NewScore, ReportRepo};
use crate::store::segments::{Segment, SegmentRepo, Speaker, ME_LABEL};
use crate::store::Store;

const NO_MEETING: &str = "This meeting no longer exists.";
const UNKNOWN_SPEAKER: &str = "Unknown speaker";

fn info(store: &Store, meeting_id: &str) -> Result<AnalysisInfo, AppError> {
    MeetingRepo::new(&*store.conn()?)
        .analysis_info(meeting_id)?
        .ok_or_else(|| AppError::NotFound(NO_MEETING.into()))
}

fn timing(info: &AnalysisInfo) -> Timing {
    Timing {
        duration_ms: info.duration_s.unwrap_or(0) * 1000,
        scheduled_ms: info.scheduled_duration_s.map(|s| s * 1000),
    }
}

/// Computes the metrics in Rust and stores them as `metric:*` scores (rule 5). No network.
pub struct MetricsStep {
    pub store: Arc<Store>,
}

impl StepRunner for MetricsStep {
    fn run(&self, meeting_id: &str) -> Result<(), AppError> {
        let timing = timing(&info(&self.store, meeting_id)?);
        let conn = self.store.conn()?;
        let segments = SegmentRepo::new(&conn).list(meeting_id)?;
        let entries = metrics::compute(&segments, timing).entries();
        ReportRepo::new(&conn).replace_metrics(meeting_id, &entries)?;
        tracing::info!(metrics = entries.len(), "metrics stored");
        Ok(())
    }
}

/// Sends the transcript and metrics to the LLM and saves the validated report. Lines are
/// cited by short refs (`s1`, ...) that map back to segment ids; an unknown ref rejects the
/// answer and the queue retries (rule 6).
pub struct AnalyzeStep {
    pub store: Arc<Store>,
    pub providers: ProviderService,
}

impl StepRunner for AnalyzeStep {
    fn run(&self, meeting_id: &str) -> Result<(), AppError> {
        let info = info(&self.store, meeting_id)?;
        let (segments, speakers) = {
            let conn = self.store.conn()?;
            let repo = SegmentRepo::new(&conn);
            (repo.list(meeting_id)?, repo.speakers(meeting_id)?)
        };
        if segments.is_empty() {
            tracing::info!("nothing was said, so there is nothing to analyze");
            return Ok(());
        }
        let metrics = metrics::compute(&segments, timing(&info));
        let request = AnalyzeRequest {
            transcript: transcript(&segments, &speakers),
            metrics: prompt_metrics(&metrics, &speakers, &info),
            template: info.template.clone(),
            highlights: Vec::new(),
        };
        let provider = self.providers.get(ProviderId::Gemini)?;
        // Runs on the jobs thread, outside the async runtime (ADR-019).
        let mut analysis = tauri::async_runtime::block_on(provider.analyze(request))?;
        let ids: HashMap<String, &str> = segments
            .iter()
            .enumerate()
            .map(|(i, s)| (line_ref(i), s.id.as_str()))
            .collect();
        analysis.resolve_segments(|r| ids.get(r).map(|id| (*id).to_owned()))?;

        let (report, actions, scores) =
            to_rows(analysis, provider.analysis_model(), &info, &metrics);
        ReportRepo::new(&*self.store.conn()?)
            .save_analysis(meeting_id, &report, &actions, &scores)?;
        tracing::info!(actions = actions.len(), "meeting analyzed");
        Ok(())
    }
}

fn line_ref(index: usize) -> String {
    format!("s{}", index + 1)
}

/// `mm:ss`, or `h:mm:ss` from an hour on.
fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Speaker names for the prompt. The user is always "Me", whatever the speaker is called.
fn names(speakers: &[Speaker]) -> HashMap<&str, &str> {
    speakers
        .iter()
        .map(|s| {
            let name = if s.is_me { ME_LABEL } else { s.label.as_str() };
            (s.id.as_str(), name)
        })
        .collect()
}

/// Lines of `[ref] [mm:ss] Speaker: text` (docs/06).
fn transcript(segments: &[Segment], speakers: &[Speaker]) -> String {
    let names = names(speakers);
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let speaker = s
                .speaker_id
                .as_deref()
                .and_then(|id| names.get(id).copied())
                .unwrap_or(UNKNOWN_SPEAKER);
            format!(
                "[{}] [{}] {speaker}: {}\n",
                line_ref(i),
                clock(s.start_ms),
                s.text.replace('\n', " ")
            )
        })
        .collect()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The metrics with speaker names instead of ids, rounded to keep the prompt short.
fn prompt_metrics(metrics: &Metrics, speakers: &[Speaker], info: &AnalysisInfo) -> Value {
    let names = names(speakers);
    let shares: Map<String, Value> = metrics
        .talk_share
        .iter()
        .map(|(id, share)| {
            let name = names.get(id.as_str()).copied().unwrap_or(UNKNOWN_SPEAKER);
            (name.to_owned(), json!(round2(*share)))
        })
        .collect();
    let mut out = Map::new();
    if let Some(s) = info.duration_s {
        out.insert("duration_min".into(), json!(round2(s as f64 / 60.0)));
    }
    out.insert("talk_share".into(), Value::Object(shares));
    for (name, value) in metrics.entries() {
        if !name.starts_with("talk_share_") {
            out.insert(name, json!(round2(value)));
        }
    }
    Value::Object(out)
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn non_blank(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// The analysis as rows. A due date that is not YYYY-MM-DD and chapters outside the
/// meeting are dropped rather than failing the whole analysis. Headline scores combine the
/// metrics with the judgements (docs/07) and keep their parts as evidence (rule 5).
fn to_rows(
    analysis: AnalysisJson,
    model: String,
    info: &AnalysisInfo,
    metrics: &Metrics,
) -> (NewReport, Vec<NewActionItem>, Vec<NewScore>) {
    let duration_ms = info.duration_s.map(|s| s * 1000);
    let chapters: Vec<_> = analysis
        .chapters
        .iter()
        .filter(|c| c.start_ms >= 0 && duration_ms.is_none_or(|d| c.start_ms <= d))
        .collect();
    let report = NewReport {
        summary_md: analysis.summary.trim().to_owned(),
        key_points: json!(analysis.key_points),
        decisions: json!(analysis.decisions),
        open_questions: json!(analysis.open_questions),
        suggestions: json!(analysis.suggestions),
        chapters: json!(chapters),
        model_used: model,
        cost_usd: None,
    };
    let actions: Vec<NewActionItem> = analysis
        .action_items
        .into_iter()
        .filter(|a| !a.task.trim().is_empty())
        .map(|a| NewActionItem {
            owner_label: non_blank(a.owner),
            task: a.task.trim().to_owned(),
            due_date: non_blank(a.due_date).filter(|d| is_date(d)),
            segment_id: a.segment_id,
        })
        .collect();
    let mut scores = vec![
        NewScore {
            kind: "metric:action_items_count".into(),
            value: actions.len() as f64,
            evidence: json!({}),
        },
        NewScore {
            kind: "metric:decisions_count".into(),
            value: analysis.decisions.len() as f64,
            evidence: json!({}),
        },
    ];
    let judged = &analysis.scores;
    let inputs = scoring::Inputs {
        metrics,
        timing: timing(info),
        template: &info.template,
        judgements: scoring::Judgements {
            value: f64::from(judged.value.score),
            engagement_quality: f64::from(judged.engagement_quality.score),
            my_contribution: f64::from(judged.my_contribution.score),
            productivity: f64::from(judged.productivity.score),
        },
        decisions: analysis.decisions.len(),
        action_items: actions.len(),
    };
    let headlines = scoring::score(&inputs, &scoring::DEFAULT_CONFIG).unwrap_or_default();
    scores.extend(headlines.into_iter().map(|h| {
        let llm = match h.kind {
            "engagement" => &judged.engagement_quality,
            "value" => &judged.value,
            "my_performance" => &judged.my_contribution,
            _ => &judged.productivity,
        };
        NewScore {
            kind: h.kind.to_owned(),
            value: h.value,
            evidence: evidence(&h, llm),
        }
    }));
    (report, actions, scores)
}

/// `{ metrics, parts, segment_ids, rationale }` (docs/05 `scores.evidence`).
fn evidence(headline: &scoring::Headline, llm: &Judged) -> Value {
    let metrics: Map<String, Value> = headline
        .parts
        .iter()
        .filter_map(|p| p.metric)
        .map(|(name, value)| (name.to_owned(), json!(value)))
        .collect();
    let parts: Vec<Value> = headline
        .parts
        .iter()
        .map(|p| json!({ "name": p.name, "weight": p.weight, "score": round2(p.score) }))
        .collect();
    json!({
        "metrics": metrics,
        "parts": parts,
        "segment_ids": llm.segment_ids,
        "rationale": llm.rationale.trim(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::capture::Track;
    use crate::providers::analysis::tests::sample;
    use crate::providers::keys::tests::MemoryApiKeys;
    use crate::providers::keys::ApiKeys;
    use crate::providers::{
        Capabilities, Provider, ProviderError, TranscribeRequest, TranscribeResult,
    };
    use crate::store::meetings::MeetingStatus;
    use crate::store::segments::NewSegment;

    type Seen = Arc<Mutex<Vec<AnalyzeRequest>>>;

    /// Answers every analysis with `answer`.
    struct FakeProvider {
        answer: Value,
        seen: Seen,
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Gemini
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                stt: true,
                diarization: true,
                llm: true,
                embeddings: false,
                offline: false,
            }
        }
        async fn check(&self) -> Result<(), ProviderError> {
            Ok(())
        }
        async fn transcribe(
            &self,
            _: TranscribeRequest,
        ) -> Result<TranscribeResult, ProviderError> {
            Ok(TranscribeResult::default())
        }
        async fn analyze(&self, req: AnalyzeRequest) -> Result<AnalysisJson, ProviderError> {
            self.seen.lock().unwrap().push(req);
            AnalysisJson::parse(&self.answer.to_string())
        }
        fn analysis_model(&self) -> String {
            "fake-model".into()
        }
        async fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
            Ok(Vec::new())
        }
        fn estimate_cost(&self, _: u32, _: u32) -> f64 {
            0.0
        }
    }

    struct Harness {
        store: Arc<Store>,
        meeting_id: String,
        seen: Seen,
    }

    impl Harness {
        /// A 10 minute meeting: s1 them, s2 me, s3 them.
        fn new() -> Self {
            let store = Arc::new(Store::open_in_memory().unwrap());
            let meeting_id = {
                let conn = store.conn().unwrap();
                let repo = MeetingRepo::new(&conn);
                let m = repo.create_recording("Planning", "zoom", 0).unwrap();
                repo.finish(&m.id, 600_000, 600, MeetingStatus::Processing)
                    .unwrap();
                let seg = |start_ms, text: &str| NewSegment {
                    start_ms,
                    end_ms: start_ms + 4_000,
                    text: text.into(),
                    speaker_label: Some("Speaker 1".into()),
                };
                let segments = SegmentRepo::new(&conn);
                segments
                    .replace_track(&m.id, Track::Mic, &[seg(5_000, "Friday works for me.")])
                    .unwrap();
                segments
                    .replace_track(
                        &m.id,
                        Track::Sys,
                        &[seg(0, "Can we ship\non Friday?"), seg(65_000, "Great.")],
                    )
                    .unwrap();
                m.id
            };
            Self {
                store,
                meeting_id,
                seen: Seen::default(),
            }
        }

        fn analyze(&self, answer: Value) -> Result<(), AppError> {
            let keys = Arc::new(MemoryApiKeys::default());
            keys.set(ProviderId::Gemini, "key").unwrap();
            let seen = Arc::clone(&self.seen);
            let providers = ProviderService::new(
                keys as Arc<dyn ApiKeys>,
                Box::new(move |_, _| {
                    Ok(Arc::new(FakeProvider {
                        answer: answer.clone(),
                        seen: Arc::clone(&seen),
                    }) as Arc<dyn Provider>)
                }),
            );
            AnalyzeStep {
                store: Arc::clone(&self.store),
                providers,
            }
            .run(&self.meeting_id)
        }

        fn segment_id(&self, index: usize) -> String {
            SegmentRepo::new(&self.store.conn().unwrap())
                .list(&self.meeting_id)
                .unwrap()[index]
                .id
                .clone()
        }

        fn count(&self, table: &str) -> i64 {
            self.store
                .conn()
                .unwrap()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE meeting_id = ?1"),
                    [&self.meeting_id],
                    |r| r.get(0),
                )
                .unwrap()
        }
    }

    #[test]
    fn rule_5_metrics_step_stores_metric_rows() {
        let h = Harness::new();
        let step = MetricsStep {
            store: Arc::clone(&h.store),
        };
        step.run(&h.meeting_id).unwrap();
        step.run(&h.meeting_id).unwrap();
        let ratio: f64 = h
            .store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM scores WHERE meeting_id = ?1 AND kind = 'metric:my_talk_ratio'",
                [&h.meeting_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((ratio - 1.0 / 3.0).abs() < 1e-9);
        assert!(h.count("scores") > 5);
        assert_eq!(step.run("gone").unwrap_err().code(), "not_found");
    }

    #[test]
    fn fr_4_1_prompt_has_refs_times_names_and_metrics() {
        let h = Harness::new();
        h.analyze(sample()).unwrap();
        let seen = h.seen.lock().unwrap();
        assert_eq!(
            seen[0].transcript,
            "[s1] [00:00] Speaker 1: Can we ship on Friday?\n\
             [s2] [00:05] Me: Friday works for me.\n\
             [s3] [01:05] Speaker 1: Great.\n"
        );
        assert_eq!(seen[0].template, "general");
        assert_eq!(seen[0].metrics["duration_min"], json!(10.0));
        assert_eq!(seen[0].metrics["talk_share"]["Me"], json!(0.33));
        assert_eq!(seen[0].metrics["questions_total"], json!(1.0));
    }

    #[test]
    fn fr_4_2_report_and_action_items_are_saved_with_segment_ids() {
        let h = Harness::new();
        let mut answer = sample();
        answer["action_items"] = json!([
            { "task": " Write notes ", "owner": "Me", "due_date": "2026-10-09", "segment_id": "s2" },
            { "task": "Book room", "owner": " ", "due_date": "next week" },
            { "task": "  " }
        ]);
        answer["chapters"] = json!([
            { "title": "Release", "start_ms": 0 },
            { "title": "Beyond the end", "start_ms": 900_000 }
        ]);
        h.analyze(answer.clone()).unwrap();

        let conn = h.store.conn().unwrap();
        let (decisions, chapters, model): (String, String, String) = conn
            .query_row(
                "SELECT decisions, chapters, model_used FROM reports WHERE meeting_id = ?1",
                [&h.meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let decisions: Value = serde_json::from_str(&decisions).unwrap();
        assert_eq!(decisions[0]["segment_id"], json!(h.segment_id(0)));
        let chapters: Value = serde_json::from_str(&chapters).unwrap();
        assert_eq!(chapters.as_array().unwrap().len(), 1);
        assert_eq!(model, "fake-model");

        let mut stmt = conn
            .prepare(
                "SELECT task, owner_label, due_date, segment_id FROM action_items
                 WHERE meeting_id = ?1 ORDER BY rowid",
            )
            .unwrap();
        type Row = (String, Option<String>, Option<String>, Option<String>);
        let items: Vec<Row> = stmt
            .query_map([&h.meeting_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            items,
            [
                (
                    "Write notes".to_owned(),
                    Some("Me".to_owned()),
                    Some("2026-10-09".to_owned()),
                    Some(h.segment_id(1))
                ),
                ("Book room".to_owned(), None, None, None),
            ]
        );
        drop(stmt);
        drop(conn);

        // A rerun replaces rather than adds.
        h.analyze(answer).unwrap();
        assert_eq!((h.count("reports"), h.count("action_items")), (1, 2));
    }

    fn headline_scores(h: &Harness) -> Vec<(String, f64, Value)> {
        let conn = h.store.conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT kind, value, evidence FROM scores
                 WHERE meeting_id = ?1 AND kind NOT LIKE 'metric:%' ORDER BY kind",
            )
            .unwrap();
        stmt.query_map([&h.meeting_id], |r| {
            let evidence: String = r.get(2)?;
            Ok((
                r.get(0)?,
                r.get(1)?,
                serde_json::from_str(&evidence).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn fr_5_1_to_5_4_headline_scores_are_saved_with_their_evidence() {
        let h = Harness::new();
        // Enough words from the other side to score (docs/07).
        let long = vec!["word"; 60].join(" ");
        SegmentRepo::new(&h.store.conn().unwrap())
            .replace_track(
                &h.meeting_id,
                Track::Sys,
                &[NewSegment {
                    start_ms: 0,
                    end_ms: 20_000,
                    text: long,
                    speaker_label: Some("Speaker 1".into()),
                }],
            )
            .unwrap();
        let mut answer = sample();
        answer["scores"]["value"] = json!({
            "score": 81, "rationale": " Clear outcome. ", "segment_ids": ["s2"]
        });
        h.analyze(answer).unwrap();

        let scores = headline_scores(&h);
        let kinds: Vec<_> = scores.iter().map(|(k, _, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            ["engagement", "my_performance", "productivity", "value"]
        );
        let (_, value, evidence) = &scores[3];
        // 60%·81 + 20%·(1/5·100) + 20%·(1/8·100) = 48.6 + 4 + 2.5, rounded
        assert_eq!(*value, 55.0);
        assert_eq!(evidence["rationale"], "Clear outcome.");
        assert_eq!(evidence["segment_ids"], json!([h.segment_id(1)]));
        assert_eq!(evidence["metrics"]["decisions_count"], json!(1.0));
        let parts: Vec<_> = evidence["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(parts, ["llm", "decisions", "action_items"]);
    }

    #[test]
    fn too_little_data_saves_the_report_without_scores() {
        let h = Harness::new();
        h.analyze(sample()).unwrap();
        assert!(headline_scores(&h).is_empty());
        assert_eq!(h.count("reports"), 1);
    }

    #[test]
    fn rule_6_an_unknown_line_saves_nothing_and_is_retryable() {
        let h = Harness::new();
        let mut answer = sample();
        answer["decisions"][0]["segment_id"] = json!("s99");
        let err = h.analyze(answer).unwrap_err();
        assert_eq!((err.code(), err.retryable()), ("invalid_llm_output", true));
        assert_eq!((h.count("reports"), h.count("scores")), (0, 0));
    }

    #[test]
    fn a_silent_meeting_is_not_sent() {
        let h = Harness::new();
        h.store
            .conn()
            .unwrap()
            .execute("DELETE FROM segments", [])
            .unwrap();
        h.analyze(sample()).unwrap();
        assert!(h.seen.lock().unwrap().is_empty());
        assert_eq!(h.count("reports"), 0);
    }

    #[test]
    fn clock_and_dates() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(125_900), "02:05");
        assert_eq!(clock(3_725_000), "1:02:05");
        for (s, ok) in [
            ("2026-10-09", true),
            ("2026-1-09", false),
            ("2026/10/09", false),
            ("Friday", false),
        ] {
            assert_eq!(is_date(s), ok, "{s}");
        }
    }
}
