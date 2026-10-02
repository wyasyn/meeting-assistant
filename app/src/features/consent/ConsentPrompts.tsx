import { useState } from "react";
import { Button } from "@/components/ui/button";
import { setAppRule, startRecording, toAppError, type MeetingDetected } from "@/lib/ipc";
import { promptText } from "./apps";
import { useConsentStore } from "./store";

/** FR-1.3: the window side of the consent prompt. Recording starts only on Record. */
export function ConsentPrompts() {
  const prompts = useConsentStore((s) => s.prompts);
  if (prompts.length === 0) return null;
  return (
    <div className="flex w-full max-w-sm flex-col gap-2">
      {prompts.map((signal) => (
        <ConsentPrompt key={signal.signalId} signal={signal} />
      ))}
    </div>
  );
}

function ConsentPrompt({ signal }: { signal: MeetingDetected }) {
  const remove = useConsentStore((s) => s.remove);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const text = promptText(signal.sourceApp);

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      remove(signal.signalId);
    } catch (e: unknown) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      aria-label="Record this meeting?"
      className="flex flex-col gap-3 rounded-lg border bg-card p-4 text-card-foreground shadow-sm"
    >
      <div>
        <h2 className="font-medium">Record this meeting?</h2>
        <p className="text-sm text-muted-foreground">{text.body}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        {/* No title: the core names the meeting after the app. */}
        <Button
          size="sm"
          disabled={busy}
          onClick={() => void run(() => startRecording({ sourceApp: signal.sourceApp }))}
        >
          Record
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => {
            remove(signal.signalId);
          }}
        >
          Not now
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={() => void run(() => setAppRule(signal.sourceApp, "never"))}
        >
          {text.never}
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
