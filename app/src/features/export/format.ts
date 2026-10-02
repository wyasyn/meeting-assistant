// Text renderings of a meeting (FR-7.1, FR-7.2). Markdown is the full record for the user,
// scores and coaching included. The Slack and email texts are for sharing with the other
// participants, so they leave out the scores and suggestions (docs/07: never score others).
import { formatElapsed } from "@/features/recording/format";
import { formatDate, ownerName, SCORE_LABELS } from "@/features/report/labels";
import { UNKNOWN_SPEAKER } from "@/features/transcript/TranscriptView";
import type { ActionItem, MeetingDetail } from "@/lib/ipc";

function when(detail: MeetingDetail): string {
  return new Date(detail.meeting.startedAt).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function participants(detail: MeetingDetail): string[] {
  return detail.speakers.map((s) => s.label);
}

/** Where a cited line is, `" (at 2:05)"`, or nothing. */
function at(detail: MeetingDetail, segmentId: string | null): string {
  const seg = segmentId === null ? undefined : detail.segments.find((s) => s.id === segmentId);
  return seg ? ` (at ${formatElapsed(seg.startMs)})` : "";
}

function actionText(item: ActionItem, owner: (label: string | null) => string | null): string {
  const extra = [owner(item.ownerLabel), item.dueDate && `due ${formatDate(item.dueDate)}`]
    .filter(Boolean)
    .join(", ");
  return extra ? `${item.task} (${extra})` : item.task;
}

/** A Markdown list, or nothing when empty. */
function mdSection(title: string, lines: string[]): string {
  return lines.length === 0 ? "" : `## ${title}\n\n${lines.join("\n")}\n\n`;
}

/** The whole meeting as a Markdown file: report, scores and transcript. */
export function toMarkdown(detail: MeetingDetail): string {
  const { meeting, report } = detail;
  const head = [
    `# ${meeting.title}`,
    "",
    `- When: ${when(detail)}`,
    ...(meeting.durationS === null ? [] : [`- Length: ${formatElapsed(meeting.durationS * 1000)}`]),
    `- Participants: ${participants(detail).join(", ")}`,
    "",
    "",
  ].join("\n");
  if (!report) return head + transcriptMd(detail);
  const scores = detail.scores.map(
    (s) =>
      `- ${SCORE_LABELS[s.kind]}: ${String(Math.round(s.value))}/100` +
      (s.evidence.rationale ? `. ${s.evidence.rationale}` : ""),
  );
  return (
    head +
    `## Summary\n\n${report.summary}\n\n` +
    mdSection("Scores", scores.length > 0 ? scores : ["Not enough data to score."]) +
    mdSection(
      "Key points",
      report.keyPoints.map((p) => `- ${p}`),
    ) +
    mdSection(
      "Decisions",
      report.decisions.map((d) => `- ${d.text}${at(detail, d.segmentId)}`),
    ) +
    mdSection(
      "Action items",
      detail.actionItems.map(
        (a) => `- [${a.done ? "x" : " "}] ${actionText(a, ownerName)}${at(detail, a.segmentId)}`,
      ),
    ) +
    mdSection(
      "Open questions",
      report.openQuestions.map((q) => `- ${q}`),
    ) +
    mdSection(
      "Suggestions for next time",
      report.suggestions.map(
        (s) => `- ${s.text} (${SCORE_LABELS[s.scoreKind]})${at(detail, s.segmentId)}`,
      ),
    ) +
    mdSection(
      "Chapters",
      report.chapters.map((c) => `- ${formatElapsed(c.startMs)} ${c.title}`),
    ) +
    transcriptMd(detail)
  );
}

function transcriptMd(detail: MeetingDetail): string {
  const names = new Map(detail.speakers.map((s) => [s.id, s.label]));
  const lines = detail.segments.map((s) => {
    const who = (s.speakerId && names.get(s.speakerId)) ?? UNKNOWN_SPEAKER;
    return `**${formatElapsed(s.startMs)} ${who}:** ${s.text}`;
  });
  return lines.length === 0 ? "" : `## Transcript\n\n${lines.join("\n\n")}\n`;
}

/** Slack mrkdwn: bold with `*`, bullets with `•`. No scores or coaching. */
export function toSlack(detail: MeetingDetail): string {
  const { report } = detail;
  const parts = [`*${detail.meeting.title}* (${when(detail)})`];
  if (report) {
    parts.push(`*Summary*\n${report.summary}`);
    const list = (title: string, items: string[]) => {
      if (items.length > 0) parts.push(`*${title}*\n${items.map((i) => `• ${i}`).join("\n")}`);
    };
    list(
      "Decisions",
      report.decisions.map((d) => d.text),
    );
    list(
      "Action items",
      detail.actionItems.map((a) => actionText(a, (l) => l)),
    );
    list("Open questions", report.openQuestions);
  }
  return `${parts.join("\n\n")}\n`;
}

/** Plain text for an email body. No scores or coaching. */
export function toEmail(detail: MeetingDetail): string {
  const { report } = detail;
  const parts = [`${detail.meeting.title}, ${when(detail)}`];
  if (report) {
    parts.push(`Summary\n${report.summary}`);
    const list = (title: string, items: string[]) => {
      if (items.length > 0) parts.push(`${title}\n${items.map((i) => `- ${i}`).join("\n")}`);
    };
    list(
      "Decisions",
      report.decisions.map((d) => d.text),
    );
    list(
      "Action items",
      detail.actionItems.map((a) => actionText(a, (l) => l)),
    );
    list("Open questions", report.openQuestions);
  }
  return `${parts.join("\n\n")}\n`;
}

/** A file name from the meeting title, without characters file systems refuse. */
export function fileName(detail: MeetingDetail, extension: string): string {
  const base = detail.meeting.title.replace(/[/\\:*?"<>|]+/g, " ").trim() || "Meeting";
  return `${base}.${extension}`;
}
