import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AppError,
  listMeetings,
  onJobProgress,
  onRecordingState,
  type JobProgress,
  type MeetingListItem,
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

function meeting(over: Partial<MeetingListItem>): MeetingListItem {
  return {
    id: "m1",
    title: "Standup",
    sourceApp: "zoom",
    startedAt: Date.UTC(2026, 9, 2, 9, 0),
    endedAt: Date.UTC(2026, 9, 2, 9, 15),
    durationS: 900,
    status: "ready",
    participants: [],
    scores: [],
    ...over,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  jobMock.mockResolvedValue(() => undefined);
  vi.mocked(onRecordingState).mockResolvedValue(() => undefined);
});

describe("MeetingList (FR-6.1)", () => {
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
    expect(screen.getByText("Processing")).toBeInTheDocument();
    expect(screen.getByText("Processing failed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Review/ })).toBeNull();
    expect(listMock).toHaveBeenCalledWith({ limit: 20 });
  });

  it("shows the source app, participants and scores", async () => {
    listMock.mockResolvedValue({
      items: [
        meeting({
          participants: ["Ann", "Bob"],
          scores: [
            { kind: "engagement", value: 71.6 },
            { kind: "value", value: 54 },
            { kind: "my_performance", value: 81 },
            { kind: "productivity", value: 66 },
          ],
        }),
        meeting({ id: "m2", title: "Notes", sourceApp: "other" }),
      ],
      nextCursor: null,
    });
    render(<MeetingList onOpen={vi.fn()} />);
    const row = await screen.findByRole("button", { name: /Standup/ });
    expect(row).toHaveTextContent("Zoom");
    expect(row).toHaveTextContent("with Ann, Bob");
    expect(screen.getByLabelText("Engagement 72")).toBeInTheDocument();
    expect(screen.getByLabelText("Value of discussion 54")).toBeInTheDocument();
    expect(screen.getByLabelText("My performance 81")).toBeInTheDocument();
    expect(screen.getByLabelText("Productivity 66")).toBeInTheDocument();
    const notes = screen.getByRole("button", { name: /Notes/ });
    expect(notes).not.toHaveTextContent("other");
    expect(notes).not.toHaveTextContent("with");
  });

  it("loads more meetings and keeps them on reload", async () => {
    let progress: ((p: JobProgress) => void) | undefined;
    jobMock.mockImplementation((handler) => {
      progress = handler;
      return Promise.resolve(() => undefined);
    });
    listMock.mockResolvedValueOnce({ items: [meeting({})], nextCursor: "c1" });
    listMock.mockResolvedValueOnce({
      items: [meeting({ id: "m2", title: "Older" })],
      nextCursor: null,
    });
    render(<MeetingList onOpen={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: "Show more" }));
    expect(await screen.findByRole("button", { name: /Older/ })).toBeInTheDocument();
    expect(listMock).toHaveBeenLastCalledWith({ cursor: "c1", limit: 20 });
    expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();

    listMock.mockResolvedValueOnce({ items: [meeting({})], nextCursor: null });
    progress?.({ meetingId: "m1", step: "analyze", status: "done", attempt: 1 });
    await screen.findByRole("button", { name: /Standup/ });
    expect(listMock).toHaveBeenLastCalledWith({ limit: 40 });
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
    expect(await screen.findByText("Processing")).toBeInTheDocument();

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
