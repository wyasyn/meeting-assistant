import { useId, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { formatElapsed } from "@/features/recording/format";
import { SCORE_KINDS, type MeetingDetail, type Score } from "@/lib/ipc";
import {
  formatCost,
  formatDate,
  lineAt,
  ownerName,
  PART_LABELS,
  partDetail,
  SCORE_LABELS,
} from "./labels";

/**
 * FR-4.3, FR-5.5: summary, decisions, action items, the four scores with why, and
 * suggestions. Cited lines link to the transcript through `onShowLine`.
 */
export function ReportView({
  detail,
  onShowLine,
}: {
  detail: MeetingDetail;
  onShowLine: (segmentId: string) => void;
}) {
  const { report, actionItems, scores } = detail;
  if (!report) {
    return (
      <p className="text-sm text-muted-foreground">
        {detail.segments.length === 0
          ? "Nobody was heard in this meeting, so there is no report."
          : "There is no report for this meeting yet."}
      </p>
    );
  }
  const starts = new Map(detail.segments.map((s) => [s.id, s.startMs]));
  const line = (segmentId: string | null) =>
    segmentId !== null && starts.has(segmentId) ? (
      <LineLink
        startMs={starts.get(segmentId) ?? 0}
        onClick={() => {
          onShowLine(segmentId);
        }}
      />
    ) : null;
  const byKind = new Map(scores.map((s) => [s.kind, s]));

  return (
    <div className="flex flex-col gap-6">
      <Section title="Summary">
        <p className="text-sm whitespace-pre-line">{report.summary}</p>
      </Section>

      <Section title="Highlights" items={detail.highlights}>
        <ul className="flex flex-col gap-1 text-sm">
          {detail.highlights.map((h) => {
            const near = lineAt(detail.segments, h.atMs);
            return (
              <li key={h.id} className="flex flex-wrap items-baseline gap-x-2">
                <span className="text-muted-foreground tabular-nums">{formatElapsed(h.atMs)}</span>
                <span>{h.note ?? "Marked moment"}</span>
                {near && <span className="text-muted-foreground">{near.text}</span>}
                {near && line(near.id)}
              </li>
            );
          })}
        </ul>
      </Section>

      <Section title="Scores">
        {scores.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            Not enough to score. Scores need a meeting of at least 5 minutes with at least 50 words
            from others.
          </p>
        ) : (
          <ul className="grid gap-2 sm:grid-cols-2">
            {SCORE_KINDS.map((kind) => {
              const score = byKind.get(kind);
              return score ? (
                <li key={kind}>
                  <ScoreCard score={score} line={line} />
                </li>
              ) : null;
            })}
          </ul>
        )}
      </Section>

      <Section title="Key points" items={report.keyPoints}>
        <ul className="list-disc pl-5 text-sm">
          {report.keyPoints.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      </Section>

      <Section title="Decisions" items={report.decisions}>
        <ul className="list-disc pl-5 text-sm">
          {report.decisions.map((d, i) => (
            <li key={i}>
              {d.text} {line(d.segmentId)}
            </li>
          ))}
        </ul>
      </Section>

      <Section title="Action items" items={actionItems}>
        <ul className="flex flex-col gap-1 text-sm">
          {actionItems.map((a) => {
            const owner = ownerName(a.ownerLabel);
            return (
              <li key={a.id}>
                <span className="font-medium">{a.task}</span>
                {owner && <span className="text-muted-foreground"> · {owner}</span>}
                {a.dueDate && (
                  <span className="text-muted-foreground"> · due {formatDate(a.dueDate)}</span>
                )}{" "}
                {line(a.segmentId)}
              </li>
            );
          })}
        </ul>
      </Section>

      <Section title="Open questions" items={report.openQuestions}>
        <ul className="list-disc pl-5 text-sm">
          {report.openQuestions.map((q, i) => (
            <li key={i}>{q}</li>
          ))}
        </ul>
      </Section>

      <Section title="Suggestions for next time" items={report.suggestions}>
        <ul className="flex flex-col gap-2 text-sm">
          {report.suggestions.map((s, i) => (
            <li key={i}>
              {s.text}{" "}
              <span className="rounded-md border px-1.5 py-0.5 text-xs text-muted-foreground">
                {SCORE_LABELS[s.scoreKind]}
              </span>{" "}
              {line(s.segmentId)}
            </li>
          ))}
        </ul>
      </Section>

      <Section title="Chapters" items={report.chapters}>
        <ol className="flex flex-col gap-1 text-sm">
          {report.chapters.map((c, i) => (
            <li key={i} className="flex gap-2">
              <span className="text-muted-foreground tabular-nums">{formatElapsed(c.startMs)}</span>
              <span>{c.title}</span>
            </li>
          ))}
        </ol>
      </Section>

      {report.costUsd !== null && (
        <p className="text-xs text-muted-foreground">
          Estimated AI cost {formatCost(report.costUsd)}, analyzed with {report.modelUsed}
        </p>
      )}
    </div>
  );
}

/** A heading and its content; left out when `items` is given and empty. */
function Section({
  title,
  items,
  children,
}: {
  title: string;
  items?: readonly unknown[];
  children: ReactNode;
}) {
  const id = useId();
  if (items?.length === 0) return null;
  return (
    <section aria-labelledby={id} className="flex flex-col gap-2">
      <h3 id={id} className="text-sm font-medium">
        {title}
      </h3>
      {children}
    </section>
  );
}

function ScoreCard({ score, line }: { score: Score; line: (segmentId: string) => ReactNode }) {
  const { evidence } = score;
  const label = SCORE_LABELS[score.kind];
  return (
    <div className="flex h-full flex-col gap-1 rounded-md border p-3">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-sm font-medium">{label}</span>
        <span
          className="text-2xl font-semibold tabular-nums"
          aria-label={`${label}: ${String(Math.round(score.value))} out of 100`}
        >
          {Math.round(score.value)}
        </span>
      </div>
      <details className="text-sm">
        <summary className="cursor-pointer text-muted-foreground">Why</summary>
        <div className="mt-2 flex flex-col gap-2">
          {evidence.rationale && <p>{evidence.rationale}</p>}
          <ul className="flex flex-col gap-0.5">
            {evidence.parts.map((p) => {
              const what = partDetail(p.name, evidence.metrics);
              return (
                <li key={p.name} className="flex justify-between gap-2">
                  <span>
                    {PART_LABELS[p.name] ?? p.name}
                    {what && <span className="text-muted-foreground"> ({what})</span>}
                  </span>
                  <span className="text-muted-foreground tabular-nums">
                    {Math.round(p.score)} × {Math.round(p.weight * 100)}%
                  </span>
                </li>
              );
            })}
          </ul>
          {evidence.segmentIds.length > 0 && (
            <p className="flex flex-wrap items-center gap-1 text-muted-foreground">
              Lines:{" "}
              {evidence.segmentIds.map((id) => (
                <span key={id}>{line(id)}</span>
              ))}
            </p>
          )}
        </div>
      </details>
    </div>
  );
}

function LineLink({ startMs, onClick }: { startMs: number; onClick: () => void }) {
  const at = formatElapsed(startMs);
  return (
    <Button
      type="button"
      variant="link"
      size="xs"
      className="h-auto px-0 tabular-nums"
      aria-label={`Show the line at ${at} in the transcript`}
      onClick={onClick}
    >
      {at}
    </Button>
  );
}
