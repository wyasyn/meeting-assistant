import { cn } from "@/lib/utils";
import { formatElapsed } from "./format";
import { useRecordingStore } from "./store";
import { useElapsed } from "./useElapsed";

/** FR-1.5 / NFR-12: the red dot and "Recording" stay on screen for the whole recording. */
export function RecordingIndicator() {
  const phase = useRecordingStore((s) => s.recording.state);
  const elapsed = useElapsed();
  if (phase !== "recording" && phase !== "paused") return null;
  const recording = phase === "recording";

  return (
    <div
      role="status"
      className="fixed inset-x-0 top-0 z-50 flex items-center justify-center gap-2 border-b bg-background/95 py-1.5 text-sm font-medium"
    >
      <span
        aria-hidden
        className={cn(
          "size-2.5 rounded-full",
          recording ? "animate-pulse bg-red-600" : "bg-muted-foreground",
        )}
      />
      <span>{recording ? "Recording" : "Paused"}</span>
      <span className="text-muted-foreground tabular-nums">{formatElapsed(elapsed)}</span>
    </div>
  );
}
