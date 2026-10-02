import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  addHighlight,
  getRecordingState,
  listAudioDevices,
  onHighlightAdded,
  pauseRecording,
  resumeRecording,
  startRecording,
  stopRecording,
  toAppError,
  type AudioDevice,
} from "@/lib/ipc";
import { defaultTitle, formatElapsed } from "./format";
import { LevelMeter } from "./LevelMeter";
import { useRecordingStore } from "./store";

/** FR-1.4: start, pause, resume and stop from the window. Recording only starts on a click. */
export function RecordingControls() {
  const recording = useRecordingStore((s) => s.recording);
  const levels = useRecordingStore((s) => s.levels);
  const apply = useRecordingStore((s) => s.apply);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const phase = recording.state;
  const active = phase === "recording" || phase === "paused";
  const marks = useHighlights();

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e: unknown) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  const start = () =>
    run(async () => {
      await startRecording({ title: defaultTitle() });
      // The `recording:state` event also carries this; reading it covers a missed event.
      apply(await getRecordingState());
    });
  const pause = () =>
    run(async () => {
      apply(await pauseRecording());
    });
  const resume = () =>
    run(async () => {
      apply(await resumeRecording());
    });
  const stop = () =>
    run(async () => {
      apply(await stopRecording());
    });
  // FR-9.1: confirmed by `highlight:added`, which the tray and `--highlight` send too.
  const highlight = () =>
    run(async () => {
      await addHighlight();
    });
  const mark = active && marks?.meetingId === recording.meetingId ? marks : null;

  const message = error ?? (phase === "stopped" ? recording.error : null);

  return (
    <section className="flex w-full max-w-sm flex-col items-center gap-4">
      {active ? (
        <div className="flex gap-2">
          {phase === "recording" ? (
            <Button variant="outline" disabled={busy} onClick={() => void pause()}>
              Pause
            </Button>
          ) : (
            <Button variant="outline" disabled={busy} onClick={() => void resume()}>
              Resume
            </Button>
          )}
          <Button variant="destructive" disabled={busy} onClick={() => void stop()}>
            Stop
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => void highlight()}>
            Add highlight
          </Button>
        </div>
      ) : (
        <Button disabled={busy} onClick={() => void start()}>
          Start recording
        </Button>
      )}

      {phase === "recording" && (
        <div className="flex w-full flex-col gap-3">
          <LevelMeter label="You" db={levels?.micDb ?? null} />
          <LevelMeter label="Others" db={levels?.sysDb ?? null} />
        </div>
      )}

      {mark && (
        <p role="status" className="text-sm text-muted-foreground">
          Highlight added at {formatElapsed(mark.atMs)}
          {mark.count > 1 && ` (${String(mark.count)} so far)`}
        </p>
      )}

      {phase === "stopped" && !message && (
        <p className="text-sm text-muted-foreground">Recording saved.</p>
      )}
      {message && (
        <p role="alert" className="text-center text-sm text-destructive">
          {message}
        </p>
      )}
      <DefaultDevices />
    </section>
  );
}

/** The latest highlight and how many the meeting has, from `highlight:added`. */
function useHighlights() {
  const [marks, setMarks] = useState<{ meetingId: string; atMs: number; count: number } | null>(
    null,
  );
  useEffect(() => {
    const unlisten = onHighlightAdded((h) => {
      setMarks((m) => ({
        meetingId: h.meetingId,
        atMs: h.atMs,
        count: m?.meetingId === h.meetingId ? m.count + 1 : 1,
      }));
    });
    return () => {
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);
  return marks;
}

/** Recording uses the system defaults for now; this shows which devices those are. */
function DefaultDevices() {
  const [devices, setDevices] = useState<AudioDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    listAudioDevices()
      .then((list) => {
        if (live) setDevices(list);
      })
      .catch((e: unknown) => {
        if (live) setError(toAppError(e).message);
      });
    return () => {
      live = false;
    };
  }, []);

  if (error) return <p className="text-center text-xs text-muted-foreground">{error}</p>;
  if (!devices) return null;
  const name = (kind: AudioDevice["kind"]) =>
    devices.find((d) => d.kind === kind && d.isDefault)?.name ?? "System default";

  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-2 text-xs text-muted-foreground">
      <dt>Microphone</dt>
      <dd>{name("input")}</dd>
      <dt>Output</dt>
      <dd>{name("output")}</dd>
    </dl>
  );
}
