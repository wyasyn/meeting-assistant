import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reportDetail } from "@/features/report/fixtures";
import { AppError, exportMeeting, pickSavePath } from "@/lib/ipc";
import { ExportActions } from "./ExportActions";
import { toMarkdown, toSlack } from "./format";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  exportMeeting: vi.fn(),
  pickSavePath: vi.fn(),
}));

const exportMock = vi.mocked(exportMeeting);
const pickMock = vi.mocked(pickSavePath);
const writeText = vi.fn<(text: string) => Promise<void>>();

beforeEach(() => {
  vi.resetAllMocks();
});

describe("ExportActions (FR-7.1, FR-7.2)", () => {
  it("saves Markdown where the user picks", async () => {
    pickMock.mockResolvedValue("/home/me/Planning.md");
    exportMock.mockResolvedValue("/home/me/Planning.md");
    render(<ExportActions detail={reportDetail()} onPrint={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Export Markdown" }));

    expect(pickMock).toHaveBeenCalledWith("Planning.md", { name: "Markdown", extensions: ["md"] });
    expect(exportMock).toHaveBeenCalledWith(
      "m1",
      "md",
      "/home/me/Planning.md",
      toMarkdown(reportDetail()),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Saved to /home/me/Planning.md");
  });

  it("does nothing when the dialog is cancelled, and shows save errors", async () => {
    pickMock.mockResolvedValueOnce(null);
    render(<ExportActions detail={reportDetail()} onPrint={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Export Markdown" }));
    expect(exportMock).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toBeEmptyDOMElement();

    pickMock.mockResolvedValueOnce("/nope/x.md");
    exportMock.mockRejectedValue(
      new AppError({ code: "storage", message: "Could not save the file.", retryable: false }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Export Markdown" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save the file.");
  });

  it("copies for Slack and says when the clipboard fails", async () => {
    // After setup, which installs its own clipboard stub.
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    writeText.mockResolvedValueOnce(undefined);
    render(<ExportActions detail={reportDetail()} onPrint={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Copy for Slack" }));
    expect(writeText).toHaveBeenCalledWith(toSlack(reportDetail()));
    expect(await screen.findByRole("status")).toHaveTextContent("Copied for Slack.");

    writeText.mockRejectedValueOnce(new Error("denied"));
    await user.click(screen.getByRole("button", { name: "Copy for email" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not copy to the clipboard.");
  });

  it("hands PDF export to the print dialog", async () => {
    const onPrint = vi.fn();
    render(<ExportActions detail={reportDetail()} onPrint={onPrint} />);
    await userEvent.click(screen.getByRole("button", { name: "Export PDF" }));
    expect(onPrint).toHaveBeenCalled();
  });
});
