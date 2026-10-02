// Words for scores, their parts and the metrics behind them (docs/07, ADR-025).
import type { ScoreKind } from "@/lib/ipc";

export const SCORE_LABELS: Record<ScoreKind, string> = {
  engagement: "Engagement",
  value: "Value of discussion",
  my_performance: "My performance",
  productivity: "Productivity",
};

export const PART_LABELS: Record<string, string> = {
  llm: "AI judgement",
  balance: "Balance of talk time",
  turns: "Back-and-forth",
  questions: "Questions asked",
  decisions: "Decisions made",
  action_items: "Action items agreed",
  talk_ratio: "My share of talk time",
  wpm: "My speaking pace",
  fillers: "My filler words",
  interruptions: "My interruptions",
  outcomes: "Outcomes for the time",
  overrun: "Finishing on time",
};

/** The metric each part is computed from, as stored in `Evidence.metrics`. */
const PART_METRIC: Record<string, string> = {
  balance: "balance",
  turns: "turns_per_min",
  questions: "questions_total",
  decisions: "decisions_count",
  action_items: "action_items_count",
  talk_ratio: "my_talk_ratio",
  wpm: "my_wpm",
  fillers: "my_filler_rate",
  interruptions: "interruptions_by_me",
  outcomes: "outcomes_count",
  overrun: "overrun_pct",
};

function count(n: number, one: string, many = `${one}s`) {
  return `${String(n)} ${n === 1 ? one : many}`;
}

const METRIC_FORMAT: Record<string, (v: number) => string> = {
  balance: (v) => `${String(Math.round(v * 100))}% even`,
  turns_per_min: (v) => `${v.toFixed(1)} speaker changes a minute`,
  questions_total: (v) => count(v, "question"),
  decisions_count: (v) => count(v, "decision"),
  action_items_count: (v) => count(v, "action item"),
  my_talk_ratio: (v) => `you spoke ${String(Math.round(v * 100))}% of the time`,
  my_wpm: (v) => `${String(Math.round(v))} words a minute`,
  my_filler_rate: (v) => `${v.toFixed(1)} filler words per 100`,
  interruptions_by_me: (v) => count(v, "interruption"),
  outcomes_count: (v) => count(v, "outcome"),
  overrun_pct: (v) => (v <= 0 ? "on time" : `${String(Math.round(v))}% over time`),
};

/** What a part measured, in words, or `null` for the AI judgement. */
export function partDetail(part: string, metrics: Record<string, number>): string | null {
  const name = PART_METRIC[part];
  const value = name === undefined ? undefined : metrics[name];
  if (name === undefined || value === undefined) return null;
  const format = METRIC_FORMAT[name];
  return format ? format(value) : String(value);
}

/** "Me" is the user. */
export function ownerName(label: string | null): string | null {
  return label === "Me" ? "You" : label;
}

/** A YYYY-MM-DD date in the local format. */
export function formatDate(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  if (!y || !m || !d) return day;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { dateStyle: "medium" });
}
