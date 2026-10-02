import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "@/App";
import { APP_NAME } from "@/config";
import {
  getRecordingState,
  onMeetingDetected,
  onMeetingEnded,
  onMeetingPromptClosed,
  onRecordingLevels,
  onRecordingState,
} from "@/lib/ipc";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  getSettings: vi.fn(() => Promise.resolve({ startOnLogin: false })),
  getRecordingState: vi.fn(() =>
    Promise.resolve({ meetingId: null, state: "idle", elapsedMs: 0, error: null }),
  ),
  listAudioDevices: vi.fn(() => Promise.resolve([])),
  onRecordingState: vi.fn(() => Promise.resolve(() => undefined)),
  onRecordingLevels: vi.fn(() => Promise.resolve(() => undefined)),
  onMeetingDetected: vi.fn(() => Promise.resolve(() => undefined)),
  onMeetingPromptClosed: vi.fn(() => Promise.resolve(() => undefined)),
  onMeetingEnded: vi.fn(() => Promise.resolve(() => undefined)),
  listAppRules: vi.fn(() => Promise.resolve([])),
  listProviders: vi.fn(() => Promise.resolve([])),
  listMeetings: vi.fn(() => Promise.resolve({ items: [], nextCursor: null })),
  onJobProgress: vi.fn(() => Promise.resolve(() => undefined)),
}));

describe("App", () => {
  it("shows the app name", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: APP_NAME })).toBeInTheDocument();
  });

  // Rule 1: nothing records on its own; the window only offers the button.
  it("offers to start a recording and follows the recording state", () => {
    render(<App />);
    expect(screen.getByRole("button", { name: "Start recording" })).toBeEnabled();
    expect(getRecordingState).toHaveBeenCalled();
    expect(onRecordingState).toHaveBeenCalled();
    expect(onRecordingLevels).toHaveBeenCalled();
  });

  // FR-1.3: detections reach the window as consent prompts.
  it("follows consent prompts", () => {
    render(<App />);
    expect(onMeetingDetected).toHaveBeenCalled();
    expect(onMeetingPromptClosed).toHaveBeenCalled();
    expect(onMeetingEnded).toHaveBeenCalled();
  });
});
