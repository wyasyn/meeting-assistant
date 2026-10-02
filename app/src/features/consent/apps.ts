import type { DetectedApp } from "@/lib/ipc";

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
