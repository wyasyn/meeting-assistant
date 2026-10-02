import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { RecordingIndicator } from "./RecordingIndicator";
import { useRecordingStore } from "./store";

function setPhase(state: "idle" | "recording" | "paused" | "stopped", elapsedMs = 0) {
  act(() => {
    useRecordingStore.getState().apply({ meetingId: "m1", state, elapsedMs, error: null });
  });
}

beforeEach(() => {
  setPhase("idle");
});

describe("RecordingIndicator (FR-1.5, NFR-12)", () => {
  it("is hidden when nothing is recorded", () => {
    render(<RecordingIndicator />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("shows the red dot label and the time while recording", () => {
    render(<RecordingIndicator />);
    setPhase("recording", 65_000);
    expect(screen.getByRole("status")).toHaveTextContent(/Recording\s*1:05/);
  });

  it("shows paused with the time so far", () => {
    render(<RecordingIndicator />);
    setPhase("paused", 5_000);
    expect(screen.getByRole("status")).toHaveTextContent(/Paused\s*0:05/);
  });

  it("goes away once stopped", () => {
    render(<RecordingIndicator />);
    setPhase("recording");
    setPhase("stopped", 5_000);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});
