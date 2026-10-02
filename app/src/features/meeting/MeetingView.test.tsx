import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reportDetail } from "@/features/report/fixtures";
import { AppError, getMeeting } from "@/lib/ipc";
import { MeetingView } from "./MeetingView";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  getMeeting: vi.fn(),
}));

const getMock = vi.mocked(getMeeting);

beforeEach(() => {
  vi.resetAllMocks();
  getMock.mockResolvedValue(reportDetail());
});

describe("MeetingView (FR-4.3)", () => {
  it("opens on the report and goes back", async () => {
    const onBack = vi.fn();
    render(<MeetingView meetingId="m1" onBack={onBack} />);
    expect(await screen.findByRole("heading", { name: "Planning" })).toBeInTheDocument();
    expect(getMock).toHaveBeenCalledWith("m1");
    expect(screen.getByRole("tab", { name: "Report" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("region", { name: "Summary" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalled();
  });

  it("a cited line opens the transcript at that line", async () => {
    render(<MeetingView meetingId="m1" onBack={vi.fn()} />);
    const decisions = await screen.findByRole("region", { name: "Decisions" });
    await userEvent.click(within(decisions).getByRole("button", { name: /line at 2:05/ }));
    expect(screen.getByRole("tab", { name: "Transcript" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("button", { name: /Friday works/ })).toHaveFocus();
  });

  it("shows why the meeting could not load", async () => {
    getMock.mockRejectedValue(
      new AppError({
        code: "not_found",
        message: "This meeting no longer exists.",
        retryable: false,
      }),
    );
    render(<MeetingView meetingId="m1" onBack={vi.fn()} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("This meeting no longer exists.");
  });
});
