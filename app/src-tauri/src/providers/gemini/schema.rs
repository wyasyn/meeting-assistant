//! `responseSchema` values in Gemini's OpenAPI subset. The analysis one mirrors the JSON
//! schema in docs/06; bounds it cannot express are checked by `AnalysisJson::parse`.

use serde_json::{json, Value};

fn string() -> Value {
    json!({ "type": "STRING" })
}

fn strings() -> Value {
    json!({ "type": "ARRAY", "items": string() })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "OBJECT", "properties": properties, "required": required })
}

pub fn transcript(diarize: bool) -> Value {
    let mut segment = json!({
        "chunk": { "type": "INTEGER" },
        "start_s": { "type": "NUMBER" },
        "end_s": { "type": "NUMBER" },
        "text": string(),
    });
    let mut required = vec!["chunk", "start_s", "end_s", "text"];
    if diarize {
        segment["speaker"] = string();
        required.push("speaker");
    }
    object(
        json!({ "segments": { "type": "ARRAY", "items": object(segment, &required) } }),
        &["segments"],
    )
}

pub fn analysis() -> Value {
    let judged = object(
        json!({
            "score": { "type": "INTEGER", "minimum": 0, "maximum": 100 },
            "rationale": string(),
            "segment_ids": strings(),
        }),
        &["score", "rationale"],
    );
    object(
        json!({
            "summary": string(),
            "key_points": strings(),
            "decisions": { "type": "ARRAY", "items": object(
                json!({ "text": string(), "segment_id": string() }), &["text"]) },
            "open_questions": strings(),
            "action_items": { "type": "ARRAY", "items": object(json!({
                "task": string(), "owner": string(), "due_date": string(), "segment_id": string(),
            }), &["task"]) },
            "scores": object(json!({
                "value": judged, "engagement_quality": judged,
                "my_contribution": judged, "productivity": judged,
            }), &["value", "engagement_quality", "my_contribution", "productivity"]),
            "suggestions": { "type": "ARRAY", "minItems": 3, "maxItems": 5, "items": object(json!({
                "text": string(),
                "score_kind": { "type": "STRING", "format": "enum",
                    "enum": ["engagement", "value", "my_performance", "productivity"] },
                "segment_id": string(),
            }), &["text", "score_kind"]) },
            "chapters": { "type": "ARRAY", "items": object(
                json!({ "title": string(), "start_ms": { "type": "INTEGER" } }),
                &["title", "start_ms"]) },
        }),
        &[
            "summary",
            "key_points",
            "decisions",
            "open_questions",
            "action_items",
            "scores",
            "suggestions",
        ],
    )
}
