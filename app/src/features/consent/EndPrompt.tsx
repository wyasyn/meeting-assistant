import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useRecordingStore } from "@/features/recording/store";
import { stopRecording, toAppError } from "@/lib/ipc";
import { endText } from "./apps";
import { useConsentStore } from "./store";

/** FR-1.6: the window side of "Meeting over?". The recording stops only on a click. */
export function EndPrompt() {
  const ending = useConsentStore((s) => s.ending);
  const remove = useConsentStore((s) => s.remove);
  const phase = useRecordingStore((s) => s.recording.state);
  const apply = useRecordingStore((s) => s.apply);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!ending || (phase !== "recording" && phase !== "paused")) return null;
  const { signalId } = ending;

  async function stop() {
    setBusy(true);
    setError(null);
    try {
      apply(await stopRecording());
      remove(signalId);
    } catch (e: unknown) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      aria-label="Meeting over?"
      className="flex w-full max-w-sm flex-col gap-3 rounded-lg border bg-card p-4 text-card-foreground shadow-sm"
    >
      <div>
        <h2 className="font-medium">Meeting over?</h2>
        <p className="text-sm text-muted-foreground">{endText(ending)}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="destructive" disabled={busy} onClick={() => void stop()}>
          Stop recording
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => {
            remove(signalId);
          }}
        >
          Keep recording
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
