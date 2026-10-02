import type { DetectedApp, MeetingEnded } from "@/lib/ipc";

/** Same names as Rust `SourceApp::label`. */
export const APP_LABELS: Record<DetectedApp, string> = {
  zoom: "Zoom",
  slack: "Slack",
  teams: "Teams",
  discord: "Discord",
  browser: "Browser",
};

/** Prompt words; the same as the notification in `src-tauri/src/consent/mod.rs`. */
export function promptText(app: DetectedApp): { body: string; never: string } {
  if (app === "browser") {
    return { body: "Your browser is using your microphone.", never: "Never for browsers" };
  }
  const label = APP_LABELS[app];
  return { body: `${label} is using your microphone.`, never: `Never for ${label}` };
}

/** End prompt words; the same as `end_note` in `src-tauri/src/consent/mod.rs`. */
export function endText({ sourceApp, reason }: Pick<MeetingEnded, "sourceApp" | "reason">): string {
  const who =
    sourceApp === null
      ? "The meeting app"
      : sourceApp === "browser"
        ? "Your browser"
        : APP_LABELS[sourceApp];
  switch (reason) {
    case "app_closed":
      return `${who} closed.`;
    case "mic_released":
      return `${who} stopped using your microphone.`;
    case "silence":
      return "No sound for 2 minutes.";
  }
}
