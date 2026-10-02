//! The analysis prompt (FR-4.1, FR-4.2, FR-5.1 to FR-5.5). Vendor-neutral: every LLM provider
//! sends the same instructions and constrains the answer to the docs/06 schema its own way.

use super::AnalyzeRequest;

const INSTRUCTIONS: &str = "\
You review a recorded meeting for the user. The user's lines are marked \"Me\". \
Each transcript line is `[ref] [time] Speaker: text`.

Rules:
- Use only what the transcript says. Never invent names, dates, numbers, decisions or tasks.
- Cite lines by their ref exactly as written in the square brackets, for example s12. \
Never invent a ref. Leave a ref out when no single line supports the point.
- Write in the language the meeting was held in.
- The metrics were computed exactly by the app. Treat them as facts; do not recompute them.
- Never judge or criticise other participants one by one. Judge the meeting as a whole, \
and the user's own part.

Fields:
- summary: 2 to 5 sentences, under 2000 characters. Start with the purpose of the meeting, \
then what came out of it.
- key_points: the main points discussed, at most 8, each one short sentence.
- decisions: only what the group clearly agreed. segment_id: the line where it was agreed.
- open_questions: questions raised and left unanswered.
- action_items: concrete tasks someone took on or was asked to do. owner: the speaker's \
name as written in the transcript (\"Me\" for the user), only when it is clear. due_date: \
YYYY-MM-DD, only when a calendar date was said; otherwise leave it out and keep any \
relative deadline (\"by Friday\") in the task. segment_id: the line where it was agreed.
- scores: integers from 0 to 100, where 50 is an ordinary meeting. Each has a one or two \
sentence rationale that names observable behaviour, and segment_ids of lines that show it.
  - value: was the discussion worth the time: new information, decisions, clarity.
  - engagement_quality: did people take part, build on each other, ask and answer.
  - my_contribution: the user's part only: clear, relevant, listening, moving things on.
  - productivity: did the meeting reach outcomes for its length and stay on topic.
- suggestions: 3 to 5 things the user can do differently next time. Each must be specific, \
actionable and tied to something observable in this meeting, never generic advice such as \
\"be more engaged\". score_kind: the score it would improve. segment_id: the line it is \
about, when there is one.
- chapters: the topics in order, each with a short title and start_ms, the start of its \
first line in milliseconds (a line at [02:05] starts at 125000). Leave chapters empty for \
meetings under 10 minutes.";

/// The full prompt: instructions, then the meeting.
pub fn analysis(req: &AnalyzeRequest) -> String {
    let highlights = if req.highlights.is_empty() {
        "none".to_owned()
    } else {
        req.highlights
            .iter()
            .map(|h| format!("\n- {h}"))
            .collect::<String>()
    };
    format!(
        "{INSTRUCTIONS}\n\n\
         Meeting type: {}\n\
         Moments the user marked as important (give them weight in the summary): {highlights}\n\
         Metrics: {}\n\n\
         Transcript:\n{}",
        req.template, req.metrics, req.transcript
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fr_4_1_prompt_carries_the_meeting_and_the_rules() {
        let req = AnalyzeRequest {
            transcript: "[s1] [00:01] Me: Ship on Friday".into(),
            metrics: json!({ "my_talk_ratio": 0.4 }),
            template: "general".into(),
            highlights: vec!["[00:01] pricing".into(), "[03:10]".into()],
        };
        let prompt = analysis(&req);
        for part in [
            "[s1] [00:01] Me: Ship on Friday",
            "\"my_talk_ratio\":0.4",
            "Meeting type: general",
            "\n- [00:01] pricing\n- [03:10]",
            "Never invent a ref",
        ] {
            assert!(prompt.contains(part), "{part}");
        }
        let none = analysis(&AnalyzeRequest {
            highlights: Vec::new(),
            ..req
        });
        assert!(none.contains("important (give them weight in the summary): none"));
    }
}
