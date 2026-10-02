import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRecordingStore } from "@/features/recording/store";
import { AppError, stopRecording, type MeetingEnded, type RecordingPhase } from "@/lib/ipc";
import { endText } from "./apps";
import { EndPrompt } from "./EndPrompt";
import { useConsentStore } from "./store";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  stopRecording: vi.fn(),
}));

const stopMock = vi.mocked(stopRecording);

const ENDED: MeetingEnded = {
  signalId: "e1",
  meetingId: "m1",
  sourceApp: "zoom",
  reason: "app_closed",
};

function recording(state: RecordingPhase) {
  useRecordingStore.getState().apply({ meetingId: "m1", state, elapsedMs: 0, error: null });
}

beforeEach(() => {
  vi.resetAllMocks();
  useConsentStore.setState({ prompts: [], ending: null });
  recording("recording");
});

describe("EndPrompt (FR-1.6)", () => {
  it("shows nothing until the meeting seems over", () => {
    const { container } = render(<EndPrompt />);
    expect(container).toBeEmptyDOMElement();
  });

  it("stops only when Stop recording is clicked", async () => {
    useConsentStore.getState().setEnding(ENDED);
    stopMock.mockResolvedValue({ meetingId: "m1", state: "stopped", elapsedMs: 1000, error: null });
    render(<EndPrompt />);
    expect(screen.getByText("Zoom closed.")).toBeInTheDocument();
    expect(stopMock).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Stop recording" }));
    expect(stopMock).toHaveBeenCalled();
    expect(useRecordingStore.getState().recording.state).toBe("stopped");
    expect(useConsentStore.getState().ending).toBeNull();
  });

  it("Keep recording closes it without stopping", async () => {
    useConsentStore.getState().setEnding(ENDED);
    render(<EndPrompt />);
    await userEvent.click(screen.getByRole("button", { name: "Keep recording" }));
    expect(stopMock).not.toHaveBeenCalled();
    expect(useConsentStore.getState().ending).toBeNull();
  });

  it("says why when stopping fails", async () => {
    useConsentStore.getState().setEnding(ENDED);
    stopMock.mockRejectedValue(
      new AppError({ code: "storage", message: "Could not save.", retryable: false }),
    );
    render(<EndPrompt />);
    await userEvent.click(screen.getByRole("button", { name: "Stop recording" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save.");
  });

  it("hides once the recording is no longer running", () => {
    useConsentStore.getState().setEnding(ENDED);
    recording("stopped");
    const { container } = render(<EndPrompt />);
    expect(container).toBeEmptyDOMElement();
  });

  it("closes on meeting:prompt-closed with its id", () => {
    useConsentStore.getState().setEnding(ENDED);
    useConsentStore.getState().remove("other");
    expect(useConsentStore.getState().ending).toEqual(ENDED);
    useConsentStore.getState().remove("e1");
    expect(useConsentStore.getState().ending).toBeNull();
  });

  it("words each reason like the notification", () => {
    expect(endText({ sourceApp: "browser", reason: "mic_released" })).toBe(
      "Your browser stopped using your microphone.",
    );
    expect(endText({ sourceApp: null, reason: "app_closed" })).toBe("The meeting app closed.");
    expect(endText({ sourceApp: "teams", reason: "silence" })).toBe("No sound for 2 minutes.");
  });
});
