import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { Button } from "@/components/ui/button";
import { formatElapsed } from "@/features/recording/format";
import {
  getSegmentAudio,
  renameSpeaker,
  toAppError,
  type MeetingDetail,
  type Speaker,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";

export const UNKNOWN_SPEAKER = "Unknown speaker";

/**
 * FR-3.1, FR-3.3, FR-3.8: the transcript with speakers and timestamps; a line plays its audio.
 * `focusSegmentId` scrolls to that line and marks it, for links from the report.
 */
export function TranscriptView({
  detail,
  onChange,
  focusSegmentId,
}: {
  detail: MeetingDetail;
  onChange: (update: (detail: MeetingDetail) => MeetingDetail) => void;
  focusSegmentId?: string | null;
}) {
  const [error, setError] = useState<string | null>(null);
  const player = usePlayer(setError);

  useEffect(() => {
    if (!focusSegmentId) return;
    const line = document.getElementById(lineId(focusSegmentId));
    line?.scrollIntoView({ block: "center" });
    line?.focus();
  }, [focusSegmentId]);

  /** The rename may have merged `from` into another speaker (same name on the same side). */
  function renamed(from: string, to: Speaker) {
    onChange((d) => ({
      ...d,
      speakers: d.speakers.flatMap((s) => (s.id === to.id ? [to] : s.id === from ? [] : [s])),
      segments: d.segments.map((seg) =>
        seg.speakerId === from ? { ...seg, speakerId: to.id } : seg,
      ),
    }));
  }

  const names = new Map(detail.speakers.map((s) => [s.id, s.label]));

  return (
    <div className="flex flex-col gap-4">
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <Speakers speakers={detail.speakers} onRenamed={renamed} onError={setError} />
      {detail.segments.length === 0 ? (
        <p className="text-sm text-muted-foreground">Nobody was heard in this meeting.</p>
      ) : (
        <ol className="flex flex-col gap-1">
          {detail.segments.map((seg) => {
            const playing = player.playing === seg.id;
            const speaker = (seg.speakerId && names.get(seg.speakerId)) ?? UNKNOWN_SPEAKER;
            return (
              <li key={seg.id}>
                <button
                  id={lineId(seg.id)}
                  type="button"
                  disabled={!detail.hasAudio}
                  aria-pressed={playing}
                  title={detail.hasAudio ? (playing ? "Stop" : "Play this line") : undefined}
                  className={cn(
                    "grid w-full grid-cols-[4rem_8rem_1fr] gap-2 rounded-md px-2 py-1 text-left text-sm focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none enabled:hover:bg-accent",
                    playing && "bg-accent",
                    focusSegmentId === seg.id && "ring-2 ring-ring",
                  )}
                  onClick={() => void player.toggle(seg.id)}
                >
                  <span className="text-muted-foreground tabular-nums">
                    {formatElapsed(seg.startMs)}
                  </span>
                  <span className="truncate font-medium">{speaker}</span>
                  <span>{seg.text}</span>
                </button>
              </li>
            );
          })}
        </ol>
      )}
      {!detail.hasAudio && (
        <p className="text-sm text-muted-foreground">
          The audio of this meeting was deleted, so lines cannot be played.
        </p>
      )}
    </div>
  );
}

function lineId(segmentId: string) {
  return `line-${segmentId}`;
}

/** One line plays at a time; clicking the playing line stops it. */
function usePlayer(onError: (message: string | null) => void) {
  const [playing, setPlaying] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const url = useRef<string | null>(null);
  /** Ignores audio that arrives after another line was clicked. */
  const request = useRef(0);

  function stop() {
    release(request, audio, url);
    setPlaying(null);
  }

  useEffect(
    () => () => {
      release(request, audio, url);
    },
    [],
  );

  async function toggle(segmentId: string) {
    const wasPlaying = playing === segmentId;
    stop();
    if (wasPlaying) return;
    const mine = request.current;
    onError(null);
    setPlaying(segmentId);
    try {
      const wav = await getSegmentAudio(segmentId);
      if (mine !== request.current) return;
      url.current = URL.createObjectURL(new Blob([wav], { type: "audio/wav" }));
      audio.current ??= new Audio();
      audio.current.src = url.current;
      audio.current.onended = () => {
        if (mine === request.current) stop();
      };
      await audio.current.play();
    } catch (e: unknown) {
      if (mine !== request.current) return;
      stop();
      onError(toAppError(e).message);
    }
  }

  return { playing, toggle };
}

/** Stops playback, frees the audio and invalidates pending requests. */
function release(
  request: RefObject<number>,
  audio: RefObject<HTMLAudioElement | null>,
  url: RefObject<string | null>,
) {
  request.current += 1;
  audio.current?.pause();
  if (url.current) URL.revokeObjectURL(url.current);
  url.current = null;
}

function Speakers({
  speakers,
  onRenamed,
  onError,
}: {
  speakers: Speaker[];
  onRenamed: (from: string, to: Speaker) => void;
  onError: (message: string | null) => void;
}) {
  const id = useId();
  if (speakers.length === 0) return null;
  return (
    <div aria-labelledby={id} role="group" className="flex flex-col gap-1">
      <h3 id={id} className="text-sm font-medium">
        Speakers
      </h3>
      <ul className="flex flex-wrap gap-2">
        {speakers.map((s) => (
          <li key={s.id}>
            <SpeakerName speaker={s} onRenamed={onRenamed} onError={onError} />
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">Give two speakers the same name to join them.</p>
    </div>
  );
}

function SpeakerName({
  speaker,
  onRenamed,
  onError,
}: {
  speaker: Speaker;
  onRenamed: (from: string, to: Speaker) => void;
  onError: (message: string | null) => void;
}) {
  const inputId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function save(name: string) {
    if (name.trim() === speaker.label) {
      setDraft(null);
      return;
    }
    setBusy(true);
    onError(null);
    try {
      onRenamed(speaker.id, await renameSpeaker(speaker.id, name));
      setDraft(null);
    } catch (e: unknown) {
      onError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  if (draft === null) {
    return (
      <span className="flex items-center gap-1 rounded-md border px-2 py-0.5 text-sm">
        <span>{speaker.label}</span>
        {speaker.isMe && <span className="text-muted-foreground">(you)</span>}
        <Button
          type="button"
          variant="link"
          size="xs"
          aria-label={`Rename ${speaker.label}`}
          onClick={() => {
            setDraft(speaker.label);
          }}
        >
          Rename
        </Button>
      </span>
    );
  }
  return (
    <form
      className="flex items-center gap-1"
      onSubmit={(e) => {
        e.preventDefault();
        void save(draft);
      }}
    >
      <label htmlFor={inputId} className="sr-only">
        New name for {speaker.label}
      </label>
      <input
        id={inputId}
        // Opened by a click on Rename, so focus follows the user's action.
        autoFocus
        className="w-36 rounded-md border bg-background px-2 py-0.5 text-sm"
        value={draft}
        disabled={busy}
        onChange={(e) => {
          setDraft(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") setDraft(null);
        }}
      />
      <Button type="submit" size="xs" disabled={busy || draft.trim() === ""}>
        Save
      </Button>
      <Button
        type="button"
        size="xs"
        variant="ghost"
        disabled={busy}
        onClick={() => {
          setDraft(null);
        }}
      >
        Cancel
      </Button>
    </form>
  );
}
