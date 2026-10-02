import { useEffect, useState } from "react";
import { useRecordingStore } from "./store";

/** Recorded time in ms, ticking while recording. Paused time is not counted (ADR-015). */
export function useElapsed(): number {
  const recording = useRecordingStore((s) => s.recording);
  const receivedAt = useRecordingStore((s) => s.receivedAt);
  const running = recording.state === "recording";
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!running) return;
    const id = setInterval(() => {
      setNow(Date.now());
    }, 250);
    return () => {
      clearInterval(id);
    };
  }, [running]);

  return running ? recording.elapsedMs + Math.max(0, now - receivedAt) : recording.elapsedMs;
}
