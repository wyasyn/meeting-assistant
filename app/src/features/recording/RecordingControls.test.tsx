import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  addHighlight,
  AppError,
  getRecordingState,
  listAudioDevices,
  onHighlightAdded,
  pauseRecording,
  startRecording,
  stopRecording,
  type Highlight,
  type RecordingState,
} from "@/lib/ipc";
import { RecordingControls } from "./RecordingControls";
import { useRecordingStore } from "./store";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  addHighlight: vi.fn(),
  getRecordingState: vi.fn(),
  listAudioDevices: vi.fn(),
  onHighlightAdded: vi.fn(),
  pauseRecording: vi.fn(),
  resumeRecording: vi.fn(),
  startRecording: vi.fn(),
  stopRecording: vi.fn(),
}));

const state = (s: RecordingState["state"], error: string | null = null): RecordingState => ({
  meetingId: "m1",
  state: s,
  elapsedMs: 0,
  error,
});

function setPhase(recording: RecordingState) {
  act(() => {
    useRecordingStore.getState().apply(recording);
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(listAudioDevices).mockResolvedValue([
    { id: "mic", name: "Built-in mic", kind: "input", isDefault: true },
    { id: "spk", name: "Speakers", kind: "output", isDefault: true },
  ]);
  vi.mocked(onHighlightAdded).mockResolvedValue(() => undefined);
  setPhase(state("idle"));
});

describe("RecordingControls (FR-1.4)", () => {
  it("starts only when the user clicks", async () => {
    vi.mocked(startRecording).mockResolvedValue({
      id: "m1",
      title: "t",
      sourceApp: "other",
      startedAt: 0,
      endedAt: null,
      durationS: null,
      status: "recording",
    });
    vi.mocked(getRecordingState).mockResolvedValue(state("recording"));
    render(<RecordingControls />);
    expect(startRecording).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Start recording" }));
    const [args] = vi.mocked(startRecording).mock.calls[0] ?? [];
    expect(args?.title).toMatch(/^Meeting on /);
    expect(await screen.findByRole("button", { name: "Pause" })).toBeInTheDocument();
    expect(screen.getByRole("meter", { name: "You" })).toBeInTheDocument();
    expect(screen.getByRole("meter", { name: "Others" })).toBeInTheDocument();
  });

  it("pauses and stops a running recording", async () => {
    setPhase(state("recording"));
    vi.mocked(pauseRecording).mockResolvedValue(state("paused"));
    vi.mocked(stopRecording).mockResolvedValue(state("stopped"));
    render(<RecordingControls />);

    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(await screen.findByRole("button", { name: "Resume" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByText("Recording saved.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start recording" })).toBeInTheDocument();
  });

  it("shows a readable error when starting fails", async () => {
    vi.mocked(startRecording).mockRejectedValue(
      new AppError({ code: "audio_device", message: "No microphone found.", retryable: false }),
    );
    render(<RecordingControls />);
    await userEvent.click(screen.getByRole("button", { name: "Start recording" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No microphone found.");
  });

  it("explains a recording that stopped by itself", () => {
    setPhase(state("stopped", "Audio capture stopped unexpectedly."));
    render(<RecordingControls />);
    expect(screen.getByRole("alert")).toHaveTextContent("Audio capture stopped unexpectedly.");
  });

  it("shows the default devices", async () => {
    render(<RecordingControls />);
    expect(await screen.findByText("Built-in mic")).toBeInTheDocument();
    expect(screen.getByText("Speakers")).toBeInTheDocument();
  });

  it("adds a highlight and confirms it from the event (FR-9.1)", async () => {
    let added: ((h: Highlight) => void) | undefined;
    vi.mocked(onHighlightAdded).mockImplementation((handler) => {
      added = handler;
      return Promise.resolve(() => undefined);
    });
    const mark = (atMs: number): Highlight => ({ id: "h", meetingId: "m1", atMs, note: null });
    vi.mocked(addHighlight).mockImplementation(() => {
      const h = mark(65_000);
      added?.(h);
      return Promise.resolve(h);
    });
    setPhase(state("recording"));
    render(<RecordingControls />);
    await userEvent.click(screen.getByRole("button", { name: "Add highlight" }));
    expect(addHighlight).toHaveBeenCalledWith();
    expect(await screen.findByRole("status")).toHaveTextContent("Highlight added at 1:05");

    // One from the tray or the command line counts too.
    act(() => {
      added?.(mark(90_000));
    });
    expect(screen.getByRole("status")).toHaveTextContent("Highlight added at 1:30 (2 so far)");

    setPhase(state("stopped"));
    expect(screen.queryByRole("status")).toBeNull();
  });
});
