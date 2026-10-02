/** `m:ss` under an hour, `h:mm:ss` after. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const ss = String(seconds).padStart(2, "0");
  if (hours > 0) return `${String(hours)}:${String(minutes).padStart(2, "0")}:${ss}`;
  return `${String(minutes)}:${ss}`;
}

/** Default title for a manual recording, in local time. */
export function defaultTitle(now: Date = new Date()): string {
  return `Meeting on ${now.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`;
}
