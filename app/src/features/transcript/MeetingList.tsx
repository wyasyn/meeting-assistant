import { useCallback, useEffect, useId, useState } from "react";
import { formatElapsed } from "@/features/recording/format";
import {
  listMeetings,
  onJobProgress,
  onRecordingState,
  toAppError,
  type MeetingSummary,
} from "@/lib/ipc";

const RECENT = 20;

const STATUS: Record<string, string> = {
  processing: "Transcribing",
  failed: "Transcription failed",
  interrupted: "Interrupted",
};

/** Recent meetings; a ready one opens its transcript. The full library is FR-6.1 (2.5). */
export function MeetingList({ onOpen }: { onOpen: (id: string) => void }) {
  const id = useId();
  const [meetings, setMeetings] = useState<MeetingSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    listMeetings({ limit: RECENT })
      .then((page) => {
        setMeetings(page.items);
        setError(null);
      })
      .catch((e: unknown) => {
        setError(toAppError(e).message);
      });
  }, []);

  useEffect(() => {
    load();
    // A stopped recording appears, and a transcribed one becomes ready, without a reload.
    const unlisten = [onRecordingState(load), onJobProgress(load)];
    return () => {
      for (const p of unlisten)
        void p.then((fn) => {
          fn();
        });
    };
  }, [load]);

  return (
    <section aria-labelledby={id} className="flex w-full max-w-sm flex-col gap-2">
      <h2 id={id} className="text-sm font-medium">
        Recent meetings
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
  meeting: MeetingSummary;
  onOpen: (id: string) => void;
}) {
  const when = new Date(meeting.startedAt).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
  const details = (
    <>
      <span className="truncate font-medium">{meeting.title}</span>
      <span className="flex items-center gap-2 text-muted-foreground">
        <span>{when}</span>
        {meeting.durationS !== null && (
          <span className="tabular-nums">{formatElapsed(meeting.durationS * 1000)}</span>
        )}
        <Status status={meeting.status} />
      </span>
    </>
  );

  if (meeting.status !== "ready") {
    return <div className="flex flex-col rounded-md px-2 py-1.5 text-sm">{details}</div>;
  }
  return (
    <button
      type="button"
      className="flex w-full flex-col rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
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
      <span className="flex items-center gap-1 text-foreground">
        <span aria-hidden className="size-2 rounded-full bg-red-600" />
        Recording
      </span>
    );
  }
  const label = STATUS[status];
  return label ? (
    <span className={status === "failed" ? "text-destructive" : ""}>{label}</span>
  ) : null;
}
