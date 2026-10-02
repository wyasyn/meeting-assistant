import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { formatElapsed } from "@/features/recording/format";
import { SCORE_LABELS } from "@/features/report/labels";
import {
  listMeetings,
  onJobProgress,
  onRecordingState,
  toAppError,
  type MeetingListItem,
} from "@/lib/ipc";

const PAGE = 20;
/** The most `list_meetings` returns at once. */
const MAX_PAGE = 200;

const STATUS: Record<string, string> = {
  processing: "Processing",
  failed: "Processing failed",
  interrupted: "Interrupted",
};

/** Where a meeting came from; `other` (started by hand) shows nothing. */
const SOURCE_LABELS: Record<string, string> = {
  zoom: "Zoom",
  meet: "Google Meet",
  slack: "Slack",
  teams: "Teams",
  discord: "Discord",
  browser: "Browser",
  in_person: "In person",
  import: "Imported",
};

/** Short score names for the row; the full name is the accessible label. */
const SHORT_LABELS: Record<string, string> = {
  engagement: "Engagement",
  value: "Value",
  my_performance: "Me",
  productivity: "Productivity",
};

/**
 * FR-6.1: every meeting, newest first, with date, length, participants, source app and
 * scores. A ready meeting opens its report.
 */
export function MeetingList({ onOpen }: { onOpen: (id: string) => void }) {
  const id = useId();
  const [meetings, setMeetings] = useState<MeetingListItem[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** How many rows are shown, so a reload keeps the pages already loaded. */
  const shown = useRef(PAGE);

  const load = useCallback(() => {
    listMeetings({ limit: Math.min(shown.current, MAX_PAGE) })
      .then((page) => {
        setMeetings(page.items);
        setCursor(page.nextCursor);
        setError(null);
      })
      .catch((e: unknown) => {
        setError(toAppError(e).message);
      });
  }, []);

  function more() {
    if (cursor === null) return;
    listMeetings({ cursor, limit: PAGE })
      .then((page) => {
        setMeetings((m) => [...(m ?? []), ...page.items]);
        setCursor(page.nextCursor);
        shown.current += PAGE;
        setError(null);
      })
      .catch((e: unknown) => {
        setError(toAppError(e).message);
      });
  }

  useEffect(() => {
    load();
    // A stopped recording appears, and a processed one becomes ready, without a reload.
    const unlisten = [onRecordingState(load), onJobProgress(load)];
    return () => {
      for (const p of unlisten)
        void p.then((fn) => {
          fn();
        });
    };
  }, [load]);

  return (
    <section aria-labelledby={id} className="flex w-full max-w-2xl flex-col gap-2">
      <h2 id={id} className="text-sm font-medium">
        Meetings
      </h2>
      {meetings?.length === 0 && (
        <p className="text-sm text-muted-foreground">Meetings you record show up here.</p>
      )}
      <ul className="flex flex-col gap-1">
        {meetings?.map((m) => (
          <li key={m.id}>
            <MeetingRow meeting={m} onOpen={onOpen} />
          </li>
        ))}
      </ul>
      {cursor !== null && (
        <Button type="button" variant="outline" size="sm" className="self-center" onClick={more}>
          Show more
        </Button>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}

function MeetingRow({
  meeting,
  onOpen,
}: {
  meeting: MeetingListItem;
  onOpen: (id: string) => void;
}) {
  const when = new Date(meeting.startedAt).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
  const source = SOURCE_LABELS[meeting.sourceApp];
  const details = (
    <>
      <span className="flex items-baseline justify-between gap-2">
        <span className="truncate font-medium">{meeting.title}</span>
        <Status status={meeting.status} />
      </span>
      <span className="flex flex-wrap items-center gap-x-2 text-muted-foreground">
        <span>{when}</span>
        {meeting.durationS !== null && (
          <span className="tabular-nums">{formatElapsed(meeting.durationS * 1000)}</span>
        )}
        {source && <span>{source}</span>}
        {meeting.participants.length > 0 && (
          <span className="truncate">with {meeting.participants.join(", ")}</span>
        )}
      </span>
      {meeting.scores.length > 0 && (
        <span className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          {meeting.scores.map((s) => (
            <span
              key={s.kind}
              aria-label={`${SCORE_LABELS[s.kind]} ${String(Math.round(s.value))}`}
            >
              {SHORT_LABELS[s.kind] ?? s.kind}{" "}
              <span className="font-medium text-foreground tabular-nums">
                {Math.round(s.value)}
              </span>
            </span>
          ))}
        </span>
      )}
    </>
  );

  if (meeting.status !== "ready") {
    return <div className="flex flex-col gap-0.5 rounded-md px-2 py-1.5 text-sm">{details}</div>;
  }
  return (
    <button
      type="button"
      className="flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
      onClick={() => {
        onOpen(meeting.id);
      }}
    >
      {details}
    </button>
  );
}

function Status({ status }: { status: string }) {
  if (status === "recording") {
    return (
      <span className="flex shrink-0 items-center gap-1 text-foreground">
        <span aria-hidden className="size-2 rounded-full bg-red-600" />
        Recording
      </span>
    );
  }
  const label = STATUS[status];
  return label ? (
    <span className={status === "failed" ? "shrink-0 text-destructive" : "shrink-0"}>{label}</span>
  ) : null;
}
