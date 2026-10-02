//! `AnalysisJson`: the LLM's analysis as typed data (docs/06 "Analysis JSON schema").
//! Field names are the schema's snake_case. Parsing plus `check` reject anything off-schema
//! (rule 6); the segment id check against the meeting joins with the analyze step (2.2).

use serde::{Deserialize, Serialize};

use super::ProviderError;

pub const SUMMARY_MAX_CHARS: usize = 2000;
pub const MIN_SUGGESTIONS: usize = 3;
pub const MAX_SUGGESTIONS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisJson {
    pub summary: String,
    pub key_points: Vec<String>,
    pub decisions: Vec<Decision>,
    pub open_questions: Vec<String>,
    pub action_items: Vec<ActionItem>,
    pub scores: Scores,
    pub suggestions: Vec<Suggestion>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub text: String,
    #[serde(default)]
    pub segment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub task: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub segment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scores {
    pub value: Judged,
    pub engagement_quality: Judged,
    pub my_contribution: Judged,
    pub productivity: Judged,
}

/// A score the LLM judged, with why (docs/07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judged {
    pub score: u8,
    pub rationale: String,
    #[serde(default)]
    pub segment_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreKind {
    Engagement,
    Value,
    MyPerformance,
    Productivity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    pub score_kind: ScoreKind,
    #[serde(default)]
    pub segment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: i64,
}

const WRONG_FORMAT: &str = "The AI returned an analysis in the wrong format.";

impl AnalysisJson {
    /// Parses the LLM's JSON text and checks the bounds the types cannot express.
    pub fn parse(text: &str) -> Result<Self, ProviderError> {
        let analysis: Self = serde_json::from_str(text).map_err(|e| {
            tracing::debug!(error = %e, "analysis did not parse");
            ProviderError::InvalidOutput(WRONG_FORMAT.into())
        })?;
        analysis.check()?;
        Ok(analysis)
    }

    fn check(&self) -> Result<(), ProviderError> {
        let scores = [
            &self.scores.value,
            &self.scores.engagement_quality,
            &self.scores.my_contribution,
            &self.scores.productivity,
        ];
        let ok = self.summary.chars().count() <= SUMMARY_MAX_CHARS
            && scores.iter().all(|s| s.score <= 100)
            && (MIN_SUGGESTIONS..=MAX_SUGGESTIONS).contains(&self.suggestions.len());
        if ok {
            Ok(())
        } else {
            tracing::debug!("analysis out of bounds");
            Err(ProviderError::InvalidOutput(WRONG_FORMAT.into()))
        }
    }
}

#[cfg(test)]
pub mod tests {
    use serde_json::{json, Value};

    use super::*;

    /// A valid analysis, for tests here and in the vendor modules.
    pub fn sample() -> Value {
        let judged = json!({ "score": 70, "rationale": "Clear agenda." });
        json!({
            "summary": "Planned the release.",
            "key_points": ["Ship on Friday"],
            "decisions": [{ "text": "Ship on Friday", "segment_id": "s1" }],
            "open_questions": [],
            "action_items": [{ "task": "Write notes", "owner": "Me" }],
            "scores": {
                "value": judged, "engagement_quality": judged,
                "my_contribution": judged, "productivity": judged
            },
            "suggestions": [
                { "text": "Share the agenda", "score_kind": "value" },
                { "text": "Ask quieter people", "score_kind": "engagement" },
                { "text": "End on time", "score_kind": "productivity" }
            ]
        })
    }

    #[test]
    fn rule_6_a_valid_analysis_parses() {
        let analysis = AnalysisJson::parse(&sample().to_string()).unwrap();
        assert_eq!(analysis.scores.value.score, 70);
        assert_eq!(analysis.action_items[0].owner.as_deref(), Some("Me"));
        assert!(analysis.chapters.is_empty());
    }

    #[test]
    fn rule_6_off_schema_analyses_are_rejected() {
        let mut no_summary = sample();
        no_summary.as_object_mut().unwrap().remove("summary");
        let mut high = sample();
        high["scores"]["value"]["score"] = json!(101);
        let mut negative = sample();
        negative["scores"]["productivity"]["score"] = json!(-1);
        let mut few = sample();
        few["suggestions"].as_array_mut().unwrap().truncate(2);
        let mut kind = sample();
        kind["suggestions"][0]["score_kind"] = json!("vibes");
        let mut long = sample();
        long["summary"] = json!("x".repeat(SUMMARY_MAX_CHARS + 1));

        for bad in [no_summary, high, negative, few, kind, long] {
            let err = AnalysisJson::parse(&bad.to_string()).unwrap_err();
            assert!(matches!(err, ProviderError::InvalidOutput(_)), "{bad}");
        }
        assert!(AnalysisJson::parse("not json").is_err());
    }
}
