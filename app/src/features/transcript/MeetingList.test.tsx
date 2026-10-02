import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AppError,
  listMeetings,
  onJobProgress,
  onRecordingState,
  type JobProgress,
  type MeetingSummary,
} from "@/lib/ipc";
import { MeetingList } from "./MeetingList";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  listMeetings: vi.fn(),
  onJobProgress: vi.fn(),
  onRecordingState: vi.fn(),
}));

const listMock = vi.mocked(listMeetings);
const jobMock = vi.mocked(onJobProgress);

function meeting(over: Partial<MeetingSummary>): MeetingSummary {
  return {
    id: "m1",
    title: "Standup",
    sourceApp: "zoom",
    startedAt: Date.UTC(2026, 9, 2, 9, 0),
    endedAt: Date.UTC(2026, 9, 2, 9, 15),
    durationS: 900,
    status: "ready",
    ...over,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  jobMock.mockResolvedValue(() => undefined);
  vi.mocked(onRecordingState).mockResolvedValue(() => undefined);
});

describe("MeetingList (FR-6.1 partial)", () => {
  it("opens a ready meeting and shows the others' status", async () => {
    listMock.mockResolvedValue({
      items: [
        meeting({}),
        meeting({ id: "m2", title: "Review", status: "processing", durationS: null }),
        meeting({ id: "m3", title: "Sync", status: "failed" }),
      ],
      nextCursor: null,
    });
    const onOpen = vi.fn();
    render(<MeetingList onOpen={onOpen} />);

    await userEvent.click(await screen.findByRole("button", { name: /Standup/ }));
    expect(onOpen).toHaveBeenCalledWith("m1");
    expect(screen.getByRole("button", { name: /Standup/ })).toHaveTextContent("15:00");
    expect(screen.getByText("Transcribing")).toBeInTheDocument();
    expect(screen.getByText("Transcription failed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Review/ })).toBeNull();
    expect(listMock).toHaveBeenCalledWith({ limit: 20 });
  });

  it("reloads when a job makes progress", async () => {
    let progress: ((p: JobProgress) => void) | undefined;
    jobMock.mockImplementation((handler) => {
      progress = handler;
      return Promise.resolve(() => undefined);
    });
    listMock.mockResolvedValueOnce({
      items: [meeting({ status: "processing" })],
      nextCursor: null,
    });
    listMock.mockResolvedValueOnce({ items: [meeting({})], nextCursor: null });
    render(<MeetingList onOpen={vi.fn()} />);
    expect(await screen.findByText("Transcribing")).toBeInTheDocument();

    progress?.({ meetingId: "m1", step: "merge", status: "done", attempt: 1 });
    expect(await screen.findByRole("button", { name: /Standup/ })).toBeInTheDocument();
  });

  it("says when there are no meetings yet, and shows errors", async () => {
    listMock.mockResolvedValueOnce({ items: [], nextCursor: null });
    render(<MeetingList onOpen={vi.fn()} />);
    expect(await screen.findByText("Meetings you record show up here.")).toBeInTheDocument();

    listMock.mockRejectedValue(
      new AppError({ code: "storage", message: "Could not read.", retryable: false }),
    );
    render(<MeetingList onOpen={vi.fn()} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not read.");
  });
});
