// A ready meeting with a full report, for tests.
import type { MeetingDetail, Score } from "@/lib/ipc";

function score(kind: Score["kind"], value: number, over: Partial<Score["evidence"]> = {}): Score {
  return {
    kind,
    value,
    evidence: {
      metrics: {},
      parts: [{ name: "llm", weight: 1, score: value }],
      segmentIds: [],
      rationale: "",
      ...over,
    },
  };
}

export function reportDetail(over: Partial<MeetingDetail> = {}): MeetingDetail {
  return {
    meeting: {
      id: "m1",
      title: "Planning",
      sourceApp: "zoom",
      startedAt: Date.UTC(2026, 9, 2, 9, 0),
      endedAt: Date.UTC(2026, 9, 2, 9, 30),
      durationS: 1800,
      status: "ready",
    },
    speakers: [
      { id: "me", label: "Me", isMe: true },
      { id: "ann", label: "Ann", isMe: false },
    ],
    segments: [
      {
        id: "a",
        track: "sys",
        speakerId: "ann",
        startMs: 1_000,
        endMs: 4_000,
        text: "Can we ship on Friday?",
      },
      {
        id: "b",
        track: "mic",
        speakerId: "me",
        startMs: 125_000,
        endMs: 128_000,
        text: "Friday works.",
      },
    ],
    hasAudio: true,
    report: {
      summary: "Planned the release.\nAgreed on Friday.",
      keyPoints: ["Release is on track"],
      decisions: [{ text: "Ship on Friday", segmentId: "b" }],
      openQuestions: ["Who writes the notes?"],
      suggestions: [
        { text: "Share the agenda first", scoreKind: "value", segmentId: null },
        { text: "Ask Ann for her view earlier", scoreKind: "engagement", segmentId: "a" },
        { text: "Close with owners", scoreKind: "productivity", segmentId: null },
      ],
      chapters: [{ title: "Release date", startMs: 0 }],
      modelUsed: "gemini-3.8-flash",
      costUsd: 0.04,
      createdAt: Date.UTC(2026, 9, 2, 9, 35),
    },
    actionItems: [
      {
        id: "i1",
        task: "Write release notes",
        ownerLabel: "Me",
        dueDate: "2026-10-09",
        done: false,
        segmentId: "b",
      },
      {
        id: "i2",
        task: "Book the demo room",
        ownerLabel: "Ann",
        dueDate: null,
        done: false,
        segmentId: null,
      },
    ],
    scores: [
      score("engagement", 72, {
        rationale: "Both people asked and answered questions.",
        metrics: { balance: 0.8, turns_per_min: 2.5 },
        parts: [
          { name: "llm", weight: 0.2, score: 60 },
          { name: "balance", weight: 0.4, score: 80 },
          { name: "turns", weight: 0.2, score: 100 },
        ],
        segmentIds: ["a"],
      }),
      score("value", 54),
      score("my_performance", 81),
      score("productivity", 66),
    ],
    highlights: [{ id: "h1", meetingId: "m1", atMs: 126_000, note: "Release date" }],
    ...over,
  };
}
