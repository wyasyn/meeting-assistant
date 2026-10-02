# 07 — Scoring

Each headline score is 0–100 and combines **deterministic metrics** (computed in `metrics/`, never by the LLM) with an **LLM judgement** (from the analysis JSON). Every score stores both in `scores.evidence`. Weights live in one config struct so they can be tuned; record changes in `09-decisions.md`.

## Deterministic metrics (stored as `metric:<name>`)
| Metric | Definition |
| --- | --- |
| `talk_share_<speaker>` | speaker talk ms / total talk ms |
| `my_talk_ratio` | my talk ms / total talk ms |
| `balance` | 1 − Gini coefficient of talk time across speakers (1 = perfectly even) |
| `turns_per_min` | speaker changes / meeting minutes |
| `longest_monologue_s` | longest continuous run by one speaker |
| `questions_total`, `my_questions` | segments ending in `?` or starting with question words |
| `interruptions_by_me`, `interruptions_of_me` | a turn starting <300 ms before the previous speaker's segment ends |
| `my_wpm` | my words / my talk minutes |
| `my_filler_rate` | filler words per 100 of my words ("um", "uh", "like", "you know", "basically", "actually", "so yeah") |
| `silence_ratio` | gaps >3 s / meeting length |
| `overrun_pct` | (actual − scheduled) / scheduled, if calendar known |
| `action_items_count`, `decisions_count` | from analysis |

## Headline scores
| Score | Formula (initial weights) |
| --- | --- |
| **Engagement** (FR-5.1) | 40% balance·100 + 20% turns-per-min band score + 20% questions band score + 20% LLM `engagement_quality` |
| **Value of discussion** (FR-5.2) | 60% LLM `value` + 20% min(decisions,5)/5·100 + 20% min(action items,8)/8·100 |
| **My performance** (FR-5.3) | 40% LLM `my_contribution` + 15% talk-ratio band + 15% wpm band (130–170 = 100) + 15% filler band (<2/100 = 100) + 15% interruptions band |
| **Productivity** (FR-5.4) | 50% LLM `productivity` + 25% outcomes per 30 min band + 25% overrun band (≤0% = 100; no calendar → re-weight to LLM) |

Band scores map a metric to 0–100 with a plateau at the healthy range and linear fall-off outside it. Healthy talk ratio for "me" depends on template: one-on-one/interview as interviewer 30–50%, client discovery 30–45%, standup ~1/N of speakers, presentation 60–80%.

## Suggestions (FR-5.5)
3–5 items, each linked to the score it would improve and, where possible, a segment. Rules for the prompt: specific, actionable next time, no generic advice ("be more engaged" is rejected), refer to observable behaviour.

## Fairness and honesty
- Show "insufficient data" instead of a score when a meeting is <5 min or has <50 words from other speakers.
- Never score other participants individually in the UI; their metrics only feed the meeting-level scores.
