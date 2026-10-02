//! Headline scores (FR-5.1 to FR-5.4): deterministic metrics combined with the LLM's
//! judgements by the weights in docs/07. Every score keeps its parts, so the UI can show
//! why (rule 5). Weights and bands live in `ScoringConfig`; changes go in 09-decisions.md.

use super::{Metrics, Timing};

/// Maps a metric to 0 to 100: 100 on the plateau `low..=high`, falling linearly to 0 at
/// `zero_low` below and `zero_high` above. Infinite ends mean no fall-off on that side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub zero_low: f64,
    pub low: f64,
    pub high: f64,
    pub zero_high: f64,
}

impl Band {
    pub fn score(&self, v: f64) -> f64 {
        let share = if v.is_nan() {
            0.0
        } else if v < self.low {
            if self.low > self.zero_low {
                (v - self.zero_low) / (self.low - self.zero_low)
            } else {
                0.0
            }
        } else if v > self.high {
            if self.zero_high > self.high {
                (self.zero_high - v) / (self.zero_high - self.high)
            } else {
                0.0
            }
        } else {
            1.0
        };
        share.clamp(0.0, 1.0) * 100.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub engagement_balance: f64,
    pub engagement_turns: f64,
    pub engagement_questions: f64,
    pub engagement_llm: f64,
    pub value_llm: f64,
    pub value_decisions: f64,
    pub value_actions: f64,
    pub my_llm: f64,
    pub my_talk_ratio: f64,
    pub my_wpm: f64,
    pub my_fillers: f64,
    pub my_interruptions: f64,
    pub productivity_llm: f64,
    pub productivity_outcomes: f64,
    pub productivity_overrun: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    pub turns_per_min: Band,
    pub questions_per_10_min: Band,
    pub wpm: Band,
    /// Fillers per 100 words.
    pub filler_rate: Band,
    /// My interruptions per 30 minutes.
    pub interruptions_per_30_min: Band,
    /// Decisions plus action items per 30 minutes.
    pub outcomes_per_30_min: Band,
    pub overrun_pct: Band,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoringConfig {
    pub weights: Weights,
    pub bands: Bands,
    /// Decisions that make the value part 100.
    pub decisions_cap: usize,
    pub actions_cap: usize,
    /// Shorter meetings get "insufficient data" instead of scores (docs/07).
    pub min_duration_ms: i64,
    /// So do meetings with fewer words from the other side.
    pub min_other_words: usize,
}

const INF: f64 = f64::INFINITY;

/// Initial weights from docs/07; band edges from ADR-025.
pub const DEFAULT_CONFIG: ScoringConfig = ScoringConfig {
    weights: Weights {
        engagement_balance: 0.40,
        engagement_turns: 0.20,
        engagement_questions: 0.20,
        engagement_llm: 0.20,
        value_llm: 0.60,
        value_decisions: 0.20,
        value_actions: 0.20,
        my_llm: 0.40,
        my_talk_ratio: 0.15,
        my_wpm: 0.15,
        my_fillers: 0.15,
        my_interruptions: 0.15,
        productivity_llm: 0.50,
        productivity_outcomes: 0.25,
        productivity_overrun: 0.25,
    },
    bands: Bands {
        turns_per_min: Band {
            zero_low: 0.0,
            low: 1.0,
            high: 6.0,
            zero_high: 15.0,
        },
        questions_per_10_min: Band {
            zero_low: 0.0,
            low: 2.0,
            high: 12.0,
            zero_high: 30.0,
        },
        wpm: Band {
            zero_low: 70.0,
            low: 130.0,
            high: 170.0,
            zero_high: 230.0,
        },
        filler_rate: Band {
            zero_low: 0.0,
            low: 0.0,
            high: 2.0,
            zero_high: 8.0,
        },
        interruptions_per_30_min: Band {
            zero_low: 0.0,
            low: 0.0,
            high: 1.0,
            zero_high: 6.0,
        },
        outcomes_per_30_min: Band {
            zero_low: 0.0,
            low: 3.0,
            high: INF,
            zero_high: INF,
        },
        overrun_pct: Band {
            zero_low: -INF,
            low: -INF,
            high: 0.0,
            zero_high: 50.0,
        },
    },
    decisions_cap: 5,
    actions_cap: 8,
    min_duration_ms: 5 * 60_000,
    min_other_words: 50,
};

/// Healthy share of talk time for the user by meeting template (docs/07). Other templates
/// use the fair share of the speakers heard, give or take 30%.
fn talk_ratio_band(template: &str, speakers: usize) -> Band {
    let (low, high) = match template {
        "one_on_one" | "interview" => (0.30, 0.50),
        "client_discovery" => (0.30, 0.45),
        "presentation" => (0.60, 0.80),
        _ => {
            let fair = 1.0 / speakers.max(1) as f64;
            ((fair * 0.7).min(1.0), (fair * 1.3).min(1.0))
        }
    };
    Band {
        zero_low: 0.0,
        low,
        high,
        zero_high: 1.0,
    }
}

/// The LLM's 0 to 100 judgements (docs/06 `scores`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Judgements {
    pub value: f64,
    pub engagement_quality: f64,
    pub my_contribution: f64,
    pub productivity: f64,
}

/// Everything a score is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs<'a> {
    pub metrics: &'a Metrics,
    pub timing: Timing,
    pub template: &'a str,
    pub judgements: Judgements,
    pub decisions: usize,
    pub action_items: usize,
}

/// One weighted part of a score.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// `llm`, `balance`, `turns`, `questions`, `decisions`, `action_items`, `talk_ratio`,
    /// `wpm`, `fillers`, `interruptions`, `outcomes`, `overrun`.
    pub name: &'static str,
    /// After re-weighting; the parts of a score add up to 1.
    pub weight: f64,
    /// 0 to 100.
    pub score: f64,
    /// The metric it came from and its raw value, when there is one.
    pub metric: Option<(&'static str, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Headline {
    /// `engagement`, `value`, `my_performance` or `productivity`.
    pub kind: &'static str,
    /// 0 to 100, whole numbers.
    pub value: f64,
    pub parts: Vec<Part>,
}

/// A part before re-weighting; `None` when its metric is undefined for the meeting.
struct Planned {
    name: &'static str,
    weight: f64,
    score: Option<f64>,
    metric: Option<(&'static str, f64)>,
}

fn part(
    name: &'static str,
    weight: f64,
    metric: (&'static str, Option<f64>),
    band: impl Fn(f64) -> f64,
) -> Planned {
    let (metric_name, value) = metric;
    Planned {
        name,
        weight,
        score: value.map(band),
        metric: value.map(|v| (metric_name, v)),
    }
}

/// Undefined parts give their weight to the LLM part (docs/07: "no calendar → re-weight to
/// LLM"), so the parts still add up to 1.
fn combine(kind: &'static str, llm: (f64, f64), planned: Vec<Planned>) -> Headline {
    let (llm_weight, llm_score) = llm;
    let spare: f64 = planned
        .iter()
        .filter(|p| p.score.is_none())
        .map(|p| p.weight)
        .sum();
    let mut parts = vec![Part {
        name: "llm",
        weight: llm_weight + spare,
        score: llm_score.clamp(0.0, 100.0),
        metric: None,
    }];
    parts.extend(planned.into_iter().filter_map(|p| {
        Some(Part {
            name: p.name,
            weight: p.weight,
            score: p.score?,
            metric: p.metric,
        })
    }));
    let value = parts.iter().map(|p| p.weight * p.score).sum::<f64>();
    Headline {
        kind,
        value: value.round().clamp(0.0, 100.0),
        parts,
    }
}

/// `None` when the meeting is too short or the other side said too little (docs/07
/// "Fairness and honesty").
pub fn score(inputs: &Inputs, config: &ScoringConfig) -> Option<Vec<Headline>> {
    let m = inputs.metrics;
    let duration_ms = inputs.timing.duration_ms;
    if duration_ms < config.min_duration_ms || m.others_word_count < config.min_other_words {
        return None;
    }
    let (w, b) = (&config.weights, &config.bands);
    let per = |count: f64, minutes: f64| count / (duration_ms as f64 / 60_000.0 / minutes);
    let capped = |n: usize, cap: usize| n.min(cap) as f64 / cap as f64 * 100.0;
    let j = inputs.judgements;

    let engagement = combine(
        "engagement",
        (w.engagement_llm, j.engagement_quality),
        vec![
            part(
                "balance",
                w.engagement_balance,
                ("balance", m.balance),
                |v| v * 100.0,
            ),
            part(
                "turns",
                w.engagement_turns,
                ("turns_per_min", m.turns_per_min),
                |v| b.turns_per_min.score(v),
            ),
            part(
                "questions",
                w.engagement_questions,
                ("questions_total", Some(f64::from(m.questions_total))),
                |v| b.questions_per_10_min.score(per(v, 10.0)),
            ),
        ],
    );
    let decisions = inputs.decisions as f64;
    let actions = inputs.action_items as f64;
    let value = combine(
        "value",
        (w.value_llm, j.value),
        vec![
            part(
                "decisions",
                w.value_decisions,
                ("decisions_count", Some(decisions)),
                |_| capped(inputs.decisions, config.decisions_cap),
            ),
            part(
                "action_items",
                w.value_actions,
                ("action_items_count", Some(actions)),
                |_| capped(inputs.action_items, config.actions_cap),
            ),
        ],
    );
    let talk_band = talk_ratio_band(inputs.template, m.talk_share.len());
    let my_performance = combine(
        "my_performance",
        (w.my_llm, j.my_contribution),
        vec![
            part(
                "talk_ratio",
                w.my_talk_ratio,
                ("my_talk_ratio", m.my_talk_ratio),
                |v| talk_band.score(v),
            ),
            part("wpm", w.my_wpm, ("my_wpm", m.my_wpm), |v| b.wpm.score(v)),
            part(
                "fillers",
                w.my_fillers,
                ("my_filler_rate", m.my_filler_rate),
                |v| b.filler_rate.score(v),
            ),
            part(
                "interruptions",
                w.my_interruptions,
                (
                    "interruptions_by_me",
                    Some(f64::from(m.interruptions_by_me)),
                ),
                |v| b.interruptions_per_30_min.score(per(v, 30.0)),
            ),
        ],
    );
    let productivity = combine(
        "productivity",
        (w.productivity_llm, j.productivity),
        vec![
            part(
                "outcomes",
                w.productivity_outcomes,
                ("outcomes_count", Some(decisions + actions)),
                |v| b.outcomes_per_30_min.score(per(v, 30.0)),
            ),
            part(
                "overrun",
                w.productivity_overrun,
                ("overrun_pct", m.overrun_pct),
                |v| b.overrun_pct.score(v),
            ),
        ],
    );
    Some(vec![engagement, value, my_performance, productivity])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn bands_plateau_and_fall_off() {
        let b = &DEFAULT_CONFIG.bands;
        let cases = [
            ("wpm on the plateau", b.wpm, 150.0, 100.0),
            ("wpm at the low edge", b.wpm, 130.0, 100.0),
            ("wpm halfway down", b.wpm, 100.0, 50.0),
            ("wpm far too slow", b.wpm, 40.0, 0.0),
            ("wpm halfway too fast", b.wpm, 200.0, 50.0),
            ("wpm far too fast", b.wpm, 300.0, 0.0),
            ("no fillers", b.filler_rate, 0.0, 100.0),
            ("five fillers per 100", b.filler_rate, 5.0, 50.0),
            ("overrun early", b.overrun_pct, -40.0, 100.0),
            ("overrun on time", b.overrun_pct, 0.0, 100.0),
            ("overrun by 25%", b.overrun_pct, 25.0, 50.0),
            ("overrun by 80%", b.overrun_pct, 80.0, 0.0),
            ("many outcomes", b.outcomes_per_30_min, 40.0, 100.0),
            ("one outcome", b.outcomes_per_30_min, 1.5, 50.0),
            ("not a number", b.wpm, f64::NAN, 0.0),
        ];
        for (name, band, v, want) in cases {
            assert!(close(band.score(v), want), "{name}: got {}", band.score(v));
        }
    }

    #[test]
    fn talk_ratio_bands_follow_the_template() {
        let cases = [
            ("interview", 2, 0.40, 100.0),
            ("interview", 2, 0.15, 50.0),
            ("client_discovery", 3, 0.45, 100.0),
            ("presentation", 5, 0.70, 100.0),
            ("presentation", 5, 0.90, 50.0),
            // Fair share of 4 speakers is 25%, healthy 17.5% to 32.5%.
            ("general", 4, 0.25, 100.0),
            ("standup", 4, 0.30, 100.0),
            ("general", 2, 0.50, 100.0),
            ("general", 2, 0.6, 100.0),
            ("general", 2, 1.0, 0.0),
        ];
        for (template, speakers, ratio, want) in cases {
            let got = talk_ratio_band(template, speakers).score(ratio);
            assert!(close(got, want), "{template} {speakers} {ratio}: got {got}");
        }
    }

    /// A healthy 30 minute meeting of two people, scheduled for 30 minutes.
    fn metrics() -> Metrics {
        Metrics {
            talk_share: BTreeMap::from([("a".into(), 0.5), ("me".into(), 0.5)]),
            my_talk_ratio: Some(0.5),
            balance: Some(1.0),
            turns_per_min: Some(3.0),
            longest_monologue_s: 60.0,
            questions_total: 15,
            my_questions: 5,
            interruptions_by_me: 0,
            interruptions_of_me: 1,
            my_wpm: Some(150.0),
            my_filler_rate: Some(1.0),
            silence_ratio: Some(0.05),
            overrun_pct: Some(0.0),
            others_word_count: 2_000,
        }
    }

    const HALF_HOUR: Timing = Timing {
        duration_ms: 30 * 60_000,
        scheduled_ms: Some(30 * 60_000),
    };

    fn inputs(metrics: &Metrics) -> Inputs<'_> {
        Inputs {
            metrics,
            timing: HALF_HOUR,
            template: "general",
            judgements: Judgements {
                value: 60.0,
                engagement_quality: 50.0,
                my_contribution: 80.0,
                productivity: 40.0,
            },
            decisions: 2,
            action_items: 4,
        }
    }

    fn by_kind(scores: &[Headline], kind: &str) -> Headline {
        scores.iter().find(|s| s.kind == kind).cloned().unwrap()
    }

    #[test]
    fn fr_5_1_to_5_4_scores_follow_the_docs_weights() {
        let m = metrics();
        let scores = score(&inputs(&m), &DEFAULT_CONFIG).unwrap();
        let kinds: Vec<_> = scores.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            ["engagement", "value", "my_performance", "productivity"]
        );
        let cases = [
            // 40%·100 + 20%·100 (3 turns/min) + 20%·100 (5 questions per 10 min) + 20%·50
            ("engagement", 90.0),
            // 60%·60 + 20%·(2/5·100) + 20%·(4/8·100) = 36 + 8 + 10
            ("value", 54.0),
            // 40%·80 + 15%·100 ×4
            ("my_performance", 92.0),
            // 50%·40 + 25%·100 (6 outcomes per 30 min) + 25%·100 (on time)
            ("productivity", 70.0),
        ];
        for (kind, want) in cases {
            let s = by_kind(&scores, kind);
            assert!(close(s.value, want), "{kind}: got {}", s.value);
            let weights: f64 = s.parts.iter().map(|p| p.weight).sum();
            assert!(close(weights, 1.0), "{kind} weights add to {weights}");
        }
        let engagement = by_kind(&scores, "engagement");
        let balance = engagement
            .parts
            .iter()
            .find(|p| p.name == "balance")
            .unwrap();
        assert_eq!(balance.metric, Some(("balance", 1.0)));
    }

    #[test]
    fn undefined_metrics_give_their_weight_to_the_llm() {
        let mut m = metrics();
        m.overrun_pct = None;
        m.my_wpm = None;
        m.my_filler_rate = None;
        let scores = score(&inputs(&m), &DEFAULT_CONFIG).unwrap();

        // No calendar: 75%·40 + 25%·100.
        let productivity = by_kind(&scores, "productivity");
        assert!(close(productivity.value, 55.0));
        let llm = &productivity.parts[0];
        assert_eq!(llm.name, "llm");
        assert!(close(llm.weight, 0.75));
        assert!(productivity.parts.iter().all(|p| p.name != "overrun"));

        // I said nothing measurable: 70%·80 + 15%·100 + 15%·100.
        let mine = by_kind(&scores, "my_performance");
        assert!(close(mine.value, 86.0));
        assert_eq!(mine.parts.len(), 3);
    }

    #[test]
    fn bad_habits_lower_my_performance() {
        let mut m = metrics();
        m.my_talk_ratio = Some(0.9);
        m.my_wpm = Some(230.0);
        m.my_filler_rate = Some(8.0);
        m.interruptions_by_me = 6;
        let scores = score(&inputs(&m), &DEFAULT_CONFIG).unwrap();
        // 40%·80 + talk ratio 0.9 against 35% to 65% (28.57) ·15%, the rest 0.
        let mine = by_kind(&scores, "my_performance");
        assert!(close(
            mine.value,
            (32.0_f64 + 0.15 * (0.1 / 0.35 * 100.0)).round()
        ));
    }

    #[test]
    fn insufficient_data_gets_no_scores() {
        let m = metrics();
        let mut short = inputs(&m);
        short.timing.duration_ms = 4 * 60_000 + 59_999;
        assert!(score(&short, &DEFAULT_CONFIG).is_none());

        let mut quiet = metrics();
        quiet.others_word_count = 49;
        assert!(score(&inputs(&quiet), &DEFAULT_CONFIG).is_none());
        quiet.others_word_count = 50;
        assert!(score(&inputs(&quiet), &DEFAULT_CONFIG).is_some());
    }

    #[test]
    fn judgements_out_of_range_are_clamped() {
        let m = metrics();
        let mut i = inputs(&m);
        i.judgements.value = 250.0;
        i.decisions = 50;
        i.action_items = 50;
        let value = by_kind(&score(&i, &DEFAULT_CONFIG).unwrap(), "value");
        assert!(close(value.value, 100.0));
    }
}
