import { render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, getSegmentAudio, renameSpeaker, type MeetingDetail } from "@/lib/ipc";
import { TranscriptView } from "./TranscriptView";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  getSegmentAudio: vi.fn(),
  renameSpeaker: vi.fn(),
}));

const audioMock = vi.mocked(getSegmentAudio);
const renameMock = vi.mocked(renameSpeaker);
const play = vi.fn<() => Promise<void>>();
const pause = vi.fn();
const revoke = vi.fn();

function detail(over: Partial<MeetingDetail> = {}): MeetingDetail {
  return {
    meeting: {
      id: "m1",
      title: "Standup",
      sourceApp: "zoom",
      startedAt: 0,
      endedAt: 60_000,
      durationS: 60,
      status: "ready",
    },
    speakers: [
      { id: "me", label: "Me", isMe: true },
      { id: "s1", label: "Speaker 1", isMe: false },
      { id: "s2", label: "Speaker 2", isMe: false },
    ],
    segments: [
      { id: "a", track: "sys", speakerId: "s1", startMs: 1_000, endMs: 2_000, text: "Hello" },
      { id: "b", track: "mic", speakerId: "me", startMs: 3_000, endMs: 4_000, text: "Hi" },
      { id: "c", track: "sys", speakerId: "s2", startMs: 65_000, endMs: 66_000, text: "Yes" },
      { id: "d", track: "sys", speakerId: null, startMs: 70_000, endMs: 71_000, text: "Hm" },
    ],
    hasAudio: true,
    report: null,
    actionItems: [],
    scores: [],
    highlights: [],
    ...over,
  };
}

/** Holds the detail the way the meeting view does. */
function View({ initial = detail(), focus }: { initial?: MeetingDetail; focus?: string }) {
  const [d, setD] = useState(initial);
  return (
    <TranscriptView
      detail={d}
      focusSegmentId={focus}
      onChange={(update) => {
        setD(update);
      }}
    />
  );
}

/** Speaker column of each line, in order. */
function lineSpeakers() {
  return screen
    .getAllByRole("button", { name: /^\d+:\d\d/ })
    .map((line) => line.children[1]?.textContent);
}

beforeEach(() => {
  vi.resetAllMocks();
  play.mockResolvedValue(undefined);
  vi.spyOn(window.HTMLMediaElement.prototype, "play").mockImplementation(play);
  vi.spyOn(window.HTMLMediaElement.prototype, "pause").mockImplementation(pause);
  URL.createObjectURL = vi.fn(() => "blob:wav");
  URL.revokeObjectURL = revoke;
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("TranscriptView (FR-3.1, FR-3.3, FR-3.8)", () => {
  it("shows speakers and timestamps, with me marked", () => {
    render(<View />);
    expect(screen.getByText("(you)")).toBeInTheDocument();
    const lines = screen.getAllByRole("button", { name: /^\d+:\d\d/ });
    expect(lines.map((l) => l.textContent)).toEqual([
      "0:01Speaker 1Hello",
      "0:03MeHi",
      "1:05Speaker 2Yes",
      "1:10Unknown speakerHm",
    ]);
  });

  it("renames a speaker once for all their lines", async () => {
    renameMock.mockResolvedValue({ id: "s1", label: "Ann", isMe: false });
    render(<View />);
    await userEvent.click(await screen.findByRole("button", { name: "Rename Speaker 1" }));
    const input = screen.getByLabelText("New name for Speaker 1");
    await userEvent.clear(input);
    await userEvent.type(input, "Ann{Enter}");

    expect(renameMock).toHaveBeenCalledWith("s1", "Ann");
    expect(await screen.findByRole("button", { name: "Rename Ann" })).toBeInTheDocument();
    expect(lineSpeakers()).toEqual(["Ann", "Me", "Speaker 2", "Unknown speaker"]);
  });

  it("merges a speaker given another speaker's name", async () => {
    renameMock.mockResolvedValue({ id: "s1", label: "Speaker 1", isMe: false });
    render(<View />);
    await userEvent.click(await screen.findByRole("button", { name: "Rename Speaker 2" }));
    const input = screen.getByLabelText("New name for Speaker 2");
    await userEvent.clear(input);
    await userEvent.type(input, "Speaker 1{Enter}");

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Rename Speaker 2" })).toBeNull();
    });
    expect(screen.getByRole("button", { name: "Rename Speaker 1" })).toBeInTheDocument();
    expect(lineSpeakers()).toEqual(["Speaker 1", "Me", "Speaker 1", "Unknown speaker"]);
  });

  it("escape cancels a rename and errors are readable", async () => {
    renameMock.mockRejectedValue(
      new AppError({ code: "invalid_state", message: "Enter a name.", retryable: false }),
    );
    render(<View />);
    await userEvent.click(await screen.findByRole("button", { name: "Rename Me" }));
    await userEvent.type(screen.getByLabelText("New name for Me"), "{Escape}");
    expect(screen.queryByLabelText("New name for Me")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Rename Me" }));
    await userEvent.type(screen.getByLabelText("New name for Me"), "x{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent("Enter a name.");
  });

  it("plays a line when clicked and stops when clicked again", async () => {
    audioMock.mockResolvedValue(new ArrayBuffer(48));
    render(<View />);
    const line = await screen.findByRole("button", { name: /Hello/ });
    await userEvent.click(line);

    expect(audioMock).toHaveBeenCalledWith("a");
    expect(play).toHaveBeenCalledTimes(1);
    expect(line).toHaveAttribute("aria-pressed", "true");

    await userEvent.click(line);
    expect(pause).toHaveBeenCalled();
    expect(revoke).toHaveBeenCalledWith("blob:wav");
    expect(line).toHaveAttribute("aria-pressed", "false");
    expect(audioMock).toHaveBeenCalledTimes(1);
  });

  it("shows why a line cannot play", async () => {
    audioMock.mockRejectedValue(
      new AppError({
        code: "storage",
        message: "A recording file could not be read. It may be damaged.",
        retryable: false,
      }),
    );
    render(<View />);
    const line = await screen.findByRole("button", { name: /Hello/ });
    await userEvent.click(line);
    expect(await screen.findByRole("alert")).toHaveTextContent("It may be damaged.");
    expect(line).toHaveAttribute("aria-pressed", "false");
  });

  it("disables playback once the audio is deleted", async () => {
    render(<View initial={detail({ hasAudio: false })} />);
    expect(await screen.findByRole("button", { name: /Hello/ })).toBeDisabled();
    expect(screen.getByText(/audio of this meeting was deleted/)).toBeInTheDocument();
  });

  it("focuses the line a report link points at", () => {
    render(<View focus="c" />);
    expect(screen.getByRole("button", { name: /Yes/ })).toHaveFocus();
  });
});
