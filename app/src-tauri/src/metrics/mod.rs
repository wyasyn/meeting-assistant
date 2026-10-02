//! Deterministic meeting metrics: talk ratio, wpm, fillers, interruptions (docs/07-scoring.md).
//! Pure functions of the transcript and the meeting's timing; the LLM never computes these
//! (rule 5). "Me" is the mic track (rule 4). See ADR-023 for how the edge cases are read.
#![allow(dead_code, reason = "the scoring step stores these from 2.3 on")]

use std::collections::BTreeMap;

use crate::capture::Track;
use crate::store::segments::Segment;

/// A turn starting more than this before the previous speaker's segment ends interrupts it.
pub const INTERRUPTION_OVERLAP_MS: i64 = 300;
/// A stretch with nobody talking counts as silence when it is longer than this.
pub const SILENCE_GAP_MS: i64 = 3_000;

const QUESTION_WORDS: &[&str] = &[
    "what", "what's", "why", "how", "how's", "when", "where", "where's", "who", "who's", "whom",
    "whose", "which",
];
const FILLERS: &[&[&str]] = &[
    &["um"],
    &["uh"],
    &["like"],
    &["basically"],
    &["actually"],
    &["you", "know"],
    &["so", "yeah"],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Recorded length of the meeting.
    pub duration_ms: i64,
    /// Length on the calendar, when known.
    pub scheduled_ms: Option<i64>,
}

/// `None` where a metric is undefined for the meeting (nobody spoke, no calendar, ...).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    /// Share of total talk time per speaker id.
    pub talk_share: BTreeMap<String, f64>,
    pub my_talk_ratio: Option<f64>,
    /// 1 − normalised Gini of talk time: 1 is even, 0 is one voice. Needs two speakers.
    pub balance: Option<f64>,
    pub turns_per_min: Option<f64>,
    pub longest_monologue_s: f64,
    pub questions_total: u32,
    pub my_questions: u32,
    pub interruptions_by_me: u32,
    pub interruptions_of_me: u32,
    pub my_wpm: Option<f64>,
    /// Filler words per 100 of my words.
    pub my_filler_rate: Option<f64>,
    pub silence_ratio: Option<f64>,
    /// Percent over the scheduled length; negative when the meeting ended early.
    pub overrun_pct: Option<f64>,
}

impl Metrics {
    /// `(name, value)` pairs for `scores` rows of kind `metric:<name>`; undefined metrics are left out.
    pub fn entries(&self) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .talk_share
            .iter()
            .map(|(speaker_id, share)| (format!("talk_share_{speaker_id}"), *share))
            .collect();
        let rest = [
            ("my_talk_ratio", self.my_talk_ratio),
            ("balance", self.balance),
            ("turns_per_min", self.turns_per_min),
            ("longest_monologue_s", Some(self.longest_monologue_s)),
            ("questions_total", Some(f64::from(self.questions_total))),
            ("my_questions", Some(f64::from(self.my_questions))),
            (
                "interruptions_by_me",
                Some(f64::from(self.interruptions_by_me)),
            ),
            (
                "interruptions_of_me",
                Some(f64::from(self.interruptions_of_me)),
            ),
            ("my_wpm", self.my_wpm),
            ("my_filler_rate", self.my_filler_rate),
            ("silence_ratio", self.silence_ratio),
            ("overrun_pct", self.overrun_pct),
        ];
        out.extend(
            rest.into_iter()
                .filter_map(|(name, value)| Some((name.to_owned(), value?))),
        );
        out
    }
}

/// A segment with a known speaker.
struct Turn<'a> {
    speaker: &'a str,
    is_me: bool,
    start_ms: i64,
    end_ms: i64,
}

fn is_me(segment: &Segment) -> bool {
    segment.track == Track::Mic.as_str()
}

pub fn compute(segments: &[Segment], timing: Timing) -> Metrics {
    let mut ordered: Vec<&Segment> = segments.iter().collect();
    ordered.sort_by_key(|s| (s.start_ms, s.end_ms));
    // Unlabelled system speech has no speaker, so it only counts as speech (silence) and
    // as a question.
    let turns: Vec<Turn> = ordered
        .iter()
        .filter_map(|s| {
            Some(Turn {
                speaker: s.speaker_id.as_deref()?,
                is_me: is_me(s),
                start_ms: s.start_ms,
                end_ms: s.end_ms,
            })
        })
        .collect();

    let mut talk: BTreeMap<&str, i64> = BTreeMap::new();
    let mut my_talk_ms = 0;
    for turn in &turns {
        let ms = (turn.end_ms - turn.start_ms).max(0);
        *talk.entry(turn.speaker).or_default() += ms;
        if turn.is_me {
            my_talk_ms += ms;
        }
    }
    let total_talk_ms: i64 = talk.values().sum();
    let talk_share = if total_talk_ms > 0 {
        talk.iter()
            .map(|(speaker, ms)| ((*speaker).to_owned(), ratio(*ms, total_talk_ms)))
            .collect()
    } else {
        BTreeMap::new()
    };

    let changes = turns
        .windows(2)
        .filter(|w| w[0].speaker != w[1].speaker)
        .count();
    let (interruptions_by_me, interruptions_of_me) = interruptions(&turns);

    let mine: Vec<&Segment> = ordered.iter().copied().filter(|s| is_me(s)).collect();
    let my_words: Vec<Vec<String>> = mine.iter().map(|s| words(&s.text)).collect();
    let my_word_count: usize = my_words.iter().map(Vec::len).sum();
    let my_fillers: usize = my_words.iter().map(|w| filler_count(w)).sum();
    let my_ms: i64 = mine.iter().map(|s| (s.end_ms - s.start_ms).max(0)).sum();

    Metrics {
        talk_share,
        my_talk_ratio: (total_talk_ms > 0).then(|| ratio(my_talk_ms, total_talk_ms)),
        balance: balance(&talk.values().copied().collect::<Vec<_>>()),
        turns_per_min: (timing.duration_ms > 0)
            .then(|| changes as f64 / minutes(timing.duration_ms)),
        longest_monologue_s: longest_monologue_ms(&turns) as f64 / 1000.0,
        questions_total: count(ordered.iter().filter(|s| is_question(&s.text))),
        my_questions: count(mine.iter().filter(|s| is_question(&s.text))),
        interruptions_by_me,
        interruptions_of_me,
        my_wpm: (my_ms > 0).then(|| my_word_count as f64 / minutes(my_ms)),
        my_filler_rate: (my_word_count > 0)
            .then(|| my_fillers as f64 * 100.0 / my_word_count as f64),
        silence_ratio: (timing.duration_ms > 0)
            .then(|| ratio(silence_ms(&ordered, timing.duration_ms), timing.duration_ms)),
        overrun_pct: timing
            .scheduled_ms
            .filter(|s| *s > 0)
            .map(|s| ratio(timing.duration_ms - s, s) * 100.0),
    }
}

fn ratio(part: i64, whole: i64) -> f64 {
    part as f64 / whole as f64
}

fn minutes(ms: i64) -> f64 {
    ms as f64 / 60_000.0
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

/// 1 − Gini, with the Gini scaled by n/(n−1) so one voice holding all the time gives 0.
fn balance(talk_ms: &[i64]) -> Option<f64> {
    let n = talk_ms.len();
    let total: i64 = talk_ms.iter().sum();
    if n < 2 || total <= 0 {
        return None;
    }
    let diff_sum: i64 = talk_ms
        .iter()
        .flat_map(|a| talk_ms.iter().map(move |b| (a - b).abs()))
        .sum();
    let gini = diff_sum as f64 / (2.0 * n as f64 * total as f64);
    let max_gini = (n - 1) as f64 / n as f64;
    Some(1.0 - gini / max_gini)
}

/// Longest stretch from a speaker's first segment to their last before someone else speaks.
fn longest_monologue_ms(turns: &[Turn]) -> i64 {
    let mut longest = 0;
    let mut run: Option<(&str, i64, i64)> = None;
    for turn in turns {
        let next = match run {
            Some((speaker, start, end)) if speaker == turn.speaker => {
                (speaker, start, end.max(turn.end_ms))
            }
            _ => (turn.speaker, turn.start_ms, turn.end_ms),
        };
        longest = longest.max(next.2 - next.1);
        run = Some(next);
    }
    longest
}

/// `(by_me, of_me)`: speaker changes that overlap the previous segment by more than
/// `INTERRUPTION_OVERLAP_MS`.
fn interruptions(turns: &[Turn]) -> (u32, u32) {
    let (mut by_me, mut of_me) = (0, 0);
    for w in turns.windows(2) {
        let (prev, cur) = (&w[0], &w[1]);
        if prev.speaker == cur.speaker || cur.start_ms >= prev.end_ms - INTERRUPTION_OVERLAP_MS {
            continue;
        }
        if cur.is_me && !prev.is_me {
            by_me += 1;
        } else if prev.is_me && !cur.is_me {
            of_me += 1;
        }
    }
    (by_me, of_me)
}

/// Lowercase words with surrounding punctuation removed.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_question(text: &str) -> bool {
    text.trim().ends_with('?')
        || words(text)
            .first()
            .is_some_and(|w| QUESTION_WORDS.contains(&w.as_str()))
}

/// Fillers within one segment; a two-word filler never spans segments.
fn filler_count(words: &[String]) -> usize {
    (0..words.len())
        .map(|i| {
            FILLERS
                .iter()
                .filter(|filler| {
                    words.len() - i >= filler.len()
                        && words[i..].iter().zip(filler.iter()).all(|(w, f)| w == f)
                })
                .count()
        })
        .sum()
}

/// Total length of gaps longer than `SILENCE_GAP_MS` with nobody talking, including before
/// the first and after the last segment. `ordered` is sorted by start.
fn silence_ms(ordered: &[&Segment], duration_ms: i64) -> i64 {
    let mut silent = 0;
    let mut cursor = 0;
    for segment in ordered {
        let start = segment.start_ms.clamp(0, duration_ms);
        if start - cursor > SILENCE_GAP_MS {
            silent += start - cursor;
        }
        cursor = cursor.max(segment.end_ms.clamp(0, duration_ms));
    }
    if duration_ms - cursor > SILENCE_GAP_MS {
        silent += duration_ms - cursor;
    }
    silent
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Timing = Timing {
        duration_ms: 60_000,
        scheduled_ms: None,
    };

    fn segment(
        track: Track,
        speaker: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        text: &str,
    ) -> Segment {
        Segment {
            id: uuid::Uuid::now_v7().to_string(),
            track: track.as_str().to_owned(),
            speaker_id: speaker.map(str::to_owned),
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    fn me(start_ms: i64, end_ms: i64, text: &str) -> Segment {
        segment(Track::Mic, Some("me"), start_ms, end_ms, text)
    }

    fn them(speaker: &str, start_ms: i64, end_ms: i64, text: &str) -> Segment {
        segment(Track::Sys, Some(speaker), start_ms, end_ms, text)
    }

    fn unlabelled(start_ms: i64, end_ms: i64, text: &str) -> Segment {
        segment(Track::Sys, None, start_ms, end_ms, text)
    }

    fn assert_close(name: &str, actual: Option<f64>, expected: Option<f64>) {
        match (actual, expected) {
            (Some(a), Some(e)) => assert!((a - e).abs() < 1e-9, "{name}: got {a}, want {e}"),
            _ => assert_eq!(actual, expected, "{name}"),
        }
    }

    #[test]
    fn talk_ratio_share_and_balance() {
        struct Case {
            name: &'static str,
            segments: Vec<Segment>,
            my_talk_ratio: Option<f64>,
            share_a: Option<f64>,
            balance: Option<f64>,
        }
        let cases = [
            Case {
                name: "even split",
                segments: vec![me(0, 10_000, "hi"), them("a", 10_000, 20_000, "hello")],
                my_talk_ratio: Some(0.5),
                share_a: Some(0.5),
                balance: Some(1.0),
            },
            Case {
                name: "three to one",
                segments: vec![me(0, 30_000, "hi"), them("a", 30_000, 40_000, "hello")],
                my_talk_ratio: Some(0.75),
                share_a: Some(0.25),
                balance: Some(0.5),
            },
            Case {
                name: "three speakers 30/10/20",
                segments: vec![
                    me(0, 30_000, "hi"),
                    them("a", 30_000, 40_000, "hello"),
                    them("b", 40_000, 60_000, "hey"),
                ],
                my_talk_ratio: Some(0.5),
                share_a: Some(10.0 / 60.0),
                balance: Some(2.0 / 3.0),
            },
            Case {
                name: "segments of one speaker add up",
                segments: vec![
                    them("a", 0, 5_000, "one"),
                    me(5_000, 15_000, "two"),
                    them("a", 15_000, 20_000, "three"),
                ],
                my_talk_ratio: Some(0.5),
                share_a: Some(0.5),
                balance: Some(1.0),
            },
            Case {
                name: "unlabelled speech is left out",
                segments: vec![
                    me(0, 10_000, "hi"),
                    unlabelled(10_000, 20_000, "noise"),
                    them("a", 20_000, 30_000, "hello"),
                ],
                my_talk_ratio: Some(0.5),
                share_a: Some(0.5),
                balance: Some(1.0),
            },
            Case {
                name: "only me",
                segments: vec![me(0, 10_000, "hi")],
                my_talk_ratio: Some(1.0),
                share_a: None,
                balance: None,
            },
            Case {
                name: "only them",
                segments: vec![them("a", 0, 10_000, "hi")],
                my_talk_ratio: Some(0.0),
                share_a: Some(1.0),
                balance: None,
            },
            Case {
                name: "nobody spoke",
                segments: vec![],
                my_talk_ratio: None,
                share_a: None,
                balance: None,
            },
        ];
        for case in cases {
            let m = compute(&case.segments, MINUTE);
            assert_close(case.name, m.my_talk_ratio, case.my_talk_ratio);
            assert_close(case.name, m.talk_share.get("a").copied(), case.share_a);
            assert_close(case.name, m.balance, case.balance);
        }
    }

    #[test]
    fn turns_and_longest_monologue() {
        struct Case {
            name: &'static str,
            segments: Vec<Segment>,
            timing: Timing,
            turns_per_min: Option<f64>,
            longest_monologue_s: f64,
        }
        let cases = [
            Case {
                name: "two changes in a minute",
                segments: vec![
                    me(0, 10_000, "a"),
                    me(10_000, 20_000, "b"),
                    them("a", 20_000, 30_000, "c"),
                    me(30_000, 40_000, "d"),
                ],
                timing: MINUTE,
                turns_per_min: Some(2.0),
                longest_monologue_s: 20.0,
            },
            Case {
                name: "input order does not matter",
                segments: vec![
                    me(30_000, 40_000, "d"),
                    them("a", 20_000, 30_000, "c"),
                    me(10_000, 20_000, "b"),
                    me(0, 10_000, "a"),
                ],
                timing: MINUTE,
                turns_per_min: Some(2.0),
                longest_monologue_s: 20.0,
            },
            Case {
                name: "rate over a two minute meeting",
                segments: vec![
                    them("a", 0, 10_000, "a"),
                    them("b", 10_000, 20_000, "b"),
                    them("a", 20_000, 30_000, "c"),
                ],
                timing: Timing {
                    duration_ms: 120_000,
                    scheduled_ms: None,
                },
                turns_per_min: Some(1.0),
                longest_monologue_s: 10.0,
            },
            Case {
                name: "a pause does not end a monologue",
                segments: vec![them("a", 0, 10_000, "a"), them("a", 15_000, 25_000, "b")],
                timing: MINUTE,
                turns_per_min: Some(0.0),
                longest_monologue_s: 25.0,
            },
            Case {
                name: "unlabelled speech does not end a monologue",
                segments: vec![
                    them("a", 0, 10_000, "a"),
                    unlabelled(10_000, 12_000, "b"),
                    them("a", 12_000, 20_000, "c"),
                ],
                timing: MINUTE,
                turns_per_min: Some(0.0),
                longest_monologue_s: 20.0,
            },
            Case {
                name: "unknown duration",
                segments: vec![me(0, 1_000, "a"), them("a", 1_000, 2_000, "b")],
                timing: Timing {
                    duration_ms: 0,
                    scheduled_ms: None,
                },
                turns_per_min: None,
                longest_monologue_s: 1.0,
            },
        ];
        for case in cases {
            let m = compute(&case.segments, case.timing);
            assert_close(case.name, m.turns_per_min, case.turns_per_min);
            assert_close(
                case.name,
                Some(m.longest_monologue_s),
                Some(case.longest_monologue_s),
            );
        }
    }

    #[test]
    fn question_detection() {
        let cases = [
            ("Can we ship on Friday?", true),
            ("  what about QA", true),
            ("What's the plan.", true),
            ("How do we start", true),
            ("Who?  ", true),
            ("So, how do we start", false),
            ("We shipped it.", false),
            ("Whatever works", false),
            ("", false),
        ];
        for (text, expected) in cases {
            assert_eq!(is_question(text), expected, "{text:?}");
        }
    }

    #[test]
    fn questions_are_counted_per_side() {
        let m = compute(
            &[
                me(0, 1_000, "Why now?"),
                me(1_000, 2_000, "Fine."),
                them("a", 2_000, 3_000, "How so?"),
                unlabelled(3_000, 4_000, "Which one"),
            ],
            MINUTE,
        );
        assert_eq!((m.questions_total, m.my_questions), (3, 1));
    }

    #[test]
    fn interruptions_by_and_of_me() {
        struct Case {
            name: &'static str,
            segments: Vec<Segment>,
            by_me: u32,
            of_me: u32,
        }
        let cases = [
            Case {
                name: "I talk over them by 1 s",
                segments: vec![them("a", 0, 10_000, "a"), me(9_000, 15_000, "b")],
                by_me: 1,
                of_me: 0,
            },
            Case {
                name: "they talk over me",
                segments: vec![me(0, 10_000, "a"), them("a", 5_000, 12_000, "b")],
                by_me: 0,
                of_me: 1,
            },
            Case {
                name: "overlap of 200 ms is a handover",
                segments: vec![them("a", 0, 10_000, "a"), me(9_800, 15_000, "b")],
                by_me: 0,
                of_me: 0,
            },
            Case {
                name: "overlap of exactly 300 ms is a handover",
                segments: vec![them("a", 0, 10_000, "a"), me(9_700, 15_000, "b")],
                by_me: 0,
                of_me: 0,
            },
            Case {
                name: "overlap of 301 ms interrupts",
                segments: vec![them("a", 0, 10_000, "a"), me(9_699, 15_000, "b")],
                by_me: 1,
                of_me: 0,
            },
            Case {
                name: "a gap is not an interruption",
                segments: vec![me(0, 10_000, "a"), them("a", 11_000, 15_000, "b")],
                by_me: 0,
                of_me: 0,
            },
            Case {
                name: "others interrupting each other are not counted",
                segments: vec![them("a", 0, 10_000, "a"), them("b", 5_000, 12_000, "b")],
                by_me: 0,
                of_me: 0,
            },
            Case {
                name: "my own overlapping segments are not counted",
                segments: vec![me(0, 10_000, "a"), me(5_000, 12_000, "b")],
                by_me: 0,
                of_me: 0,
            },
        ];
        for case in cases {
            let m = compute(&case.segments, MINUTE);
            assert_eq!(
                (m.interruptions_by_me, m.interruptions_of_me),
                (case.by_me, case.of_me),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn words_per_minute() {
        let words = |n: usize| vec!["word"; n].join(" ");
        let cases = [
            (
                "150 words in a minute",
                vec![me(0, 60_000, &words(150))],
                Some(150.0),
            ),
            (
                "60 words in 30 s",
                vec![me(0, 30_000, &words(60))],
                Some(120.0),
            ),
            (
                "split over two segments",
                vec![me(0, 15_000, &words(30)), me(40_000, 55_000, &words(30))],
                Some(120.0),
            ),
            (
                "others are left out",
                vec![them("a", 0, 60_000, &words(150))],
                None,
            ),
        ];
        for (name, segments, expected) in cases {
            assert_close(name, compute(&segments, MINUTE).my_wpm, expected);
        }
    }

    #[test]
    fn filler_rate() {
        let cases = [
            (
                "four fillers in nine words",
                vec![me(
                    0,
                    5_000,
                    "Um, I think, you know, it's like basically done",
                )],
                Some(400.0 / 9.0),
            ),
            (
                "so yeah and actually",
                vec![me(0, 5_000, "So yeah. Actually no.")],
                Some(50.0),
            ),
            ("no fillers", vec![me(0, 5_000, "We ship today")], Some(0.0)),
            (
                "two-word fillers do not span segments",
                vec![me(0, 1_000, "you"), me(1_000, 2_000, "know")],
                Some(0.0),
            ),
            (
                "their fillers are left out",
                vec![me(0, 1_000, "Done."), them("a", 1_000, 2_000, "um uh like")],
                Some(0.0),
            ),
            ("I said nothing", vec![them("a", 0, 1_000, "um")], None),
        ];
        for (name, segments, expected) in cases {
            assert_close(name, compute(&segments, MINUTE).my_filler_rate, expected);
        }
    }

    #[test]
    fn silence_ratio() {
        let cases = [
            ("all talk", vec![me(0, 60_000, "a")], MINUTE, Some(0.0)),
            (
                "a 4 s gap",
                vec![me(0, 10_000, "a"), them("a", 14_000, 60_000, "b")],
                MINUTE,
                Some(4.0 / 60.0),
            ),
            (
                "a 3 s gap is not silence",
                vec![me(0, 10_000, "a"), them("a", 13_000, 60_000, "b")],
                MINUTE,
                Some(0.0),
            ),
            (
                "silence before and after",
                vec![me(5_000, 20_000, "a")],
                MINUTE,
                Some(0.75),
            ),
            (
                "overlapping speech has no gaps",
                vec![
                    me(0, 30_000, "a"),
                    them("a", 10_000, 20_000, "b"),
                    them("b", 25_000, 60_000, "c"),
                ],
                MINUTE,
                Some(0.0),
            ),
            (
                "unlabelled speech is not silence",
                vec![unlabelled(0, 60_000, "a")],
                MINUTE,
                Some(0.0),
            ),
            (
                "segments past the end are clamped",
                vec![me(0, 90_000, "a")],
                MINUTE,
                Some(0.0),
            ),
            ("no speech at all", vec![], MINUTE, Some(1.0)),
            (
                "unknown duration",
                vec![me(0, 1_000, "a")],
                Timing {
                    duration_ms: 0,
                    scheduled_ms: None,
                },
                None,
            ),
        ];
        for (name, segments, timing, expected) in cases {
            assert_close(name, compute(&segments, timing).silence_ratio, expected);
        }
    }

    #[test]
    fn overrun() {
        let cases = [
            ("20% over", 60_000, Some(50_000), Some(20.0)),
            ("ended early", 45_000, Some(60_000), Some(-25.0)),
            ("on time", 60_000, Some(60_000), Some(0.0)),
            ("no calendar", 60_000, None, None),
            ("zero scheduled length", 60_000, Some(0), None),
        ];
        for (name, duration_ms, scheduled_ms, expected) in cases {
            let timing = Timing {
                duration_ms,
                scheduled_ms,
            };
            assert_close(name, compute(&[], timing).overrun_pct, expected);
        }
    }

    #[test]
    fn entries_name_every_defined_metric() {
        let m = compute(
            &[me(0, 30_000, "Why?"), them("a", 30_000, 60_000, "Because.")],
            MINUTE,
        );
        let names: Vec<String> = m.entries().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "talk_share_a",
                "talk_share_me",
                "my_talk_ratio",
                "balance",
                "turns_per_min",
                "longest_monologue_s",
                "questions_total",
                "my_questions",
                "interruptions_by_me",
                "interruptions_of_me",
                "my_wpm",
                "my_filler_rate",
                "silence_ratio",
            ]
        );
    }
}
