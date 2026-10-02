import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { reportDetail } from "./fixtures";
import { formatCost, formatDate, lineAt, partDetail } from "./labels";
import { ReportView } from "./ReportView";

function fail(): never {
  throw new Error("missing element");
}

function section(name: string) {
  return screen.getByRole("region", { name });
}

describe("ReportView (FR-4.3, FR-5.5)", () => {
  it("shows the summary, decisions, action items and open questions", () => {
    render(<ReportView detail={reportDetail()} onShowLine={vi.fn()} />);
    expect(within(section("Summary")).getByText(/Planned the release/)).toBeInTheDocument();
    expect(within(section("Key points")).getByText("Release is on track")).toBeInTheDocument();
    expect(within(section("Decisions")).getByText(/Ship on Friday/)).toBeInTheDocument();
    const actions = within(section("Action items")).getAllByRole("listitem");
    expect(actions[0]).toHaveTextContent(
      `Write release notes · You · due ${formatDate("2026-10-09")}`,
    );
    expect(actions[1]).toHaveTextContent("Book the demo room · Ann");
    expect(
      within(section("Open questions")).getByText("Who writes the notes?"),
    ).toBeInTheDocument();
    expect(within(section("Chapters")).getByText("Release date")).toBeInTheDocument();
  });

  it("shows the four scores and why", async () => {
    render(<ReportView detail={reportDetail()} onShowLine={vi.fn()} />);
    const scores = section("Scores");
    for (const [label, value] of [
      ["Engagement", 72],
      ["Value of discussion", 54],
      ["My performance", 81],
      ["Productivity", 66],
    ] as const) {
      expect(
        within(scores).getByLabelText(`${label}: ${String(value)} out of 100`),
      ).toBeInTheDocument();
    }
    await userEvent.click(within(scores).getAllByText("Why")[0] ?? fail());
    expect(screen.getByText("Both people asked and answered questions.")).toBeVisible();
    expect(screen.getByText("Balance of talk time")).toBeInTheDocument();
    expect(screen.getByText("(80% even)")).toBeInTheDocument();
    expect(screen.getByText("80 × 40%")).toBeInTheDocument();
  });

  it("ties suggestions to a score and links cited lines to the transcript", async () => {
    const onShowLine = vi.fn();
    render(<ReportView detail={reportDetail()} onShowLine={onShowLine} />);
    const suggestions = within(section("Suggestions for next time")).getAllByRole("listitem");
    expect(suggestions).toHaveLength(3);
    expect(suggestions[1]).toHaveTextContent("Ask Ann for her view earlier");
    expect(suggestions[1]).toHaveTextContent("Engagement");

    await userEvent.click(
      within(suggestions[1] ?? fail()).getByRole("button", {
        name: "Show the line at 0:01 in the transcript",
      }),
    );
    expect(onShowLine).toHaveBeenCalledWith("a");
    await userEvent.click(
      within(section("Decisions")).getByRole("button", { name: /line at 2:05/ }),
    );
    expect(onShowLine).toHaveBeenLastCalledWith("b");
  });

  it("pins highlights with their line and shows the cost (FR-9.1, FR-8.4)", async () => {
    const onShowLine = vi.fn();
    render(<ReportView detail={reportDetail()} onShowLine={onShowLine} />);
    const highlights = within(section("Highlights")).getAllByRole("listitem");
    expect(highlights[0]).toHaveTextContent("2:06Release dateFriday works.");
    await userEvent.click(
      within(section("Highlights")).getByRole("button", { name: /line at 2:05/ }),
    );
    expect(onShowLine).toHaveBeenCalledWith("b");
    expect(
      screen.getByText("Estimated AI cost $0.04, analyzed with gemini-3.8-flash"),
    ).toBeInTheDocument();
  });

  it("says when there was too little data to score", () => {
    render(<ReportView detail={reportDetail({ scores: [] })} onShowLine={vi.fn()} />);
    expect(within(section("Scores")).getByText(/Not enough to score/)).toBeInTheDocument();
  });

  it("explains a missing report and leaves out empty sections", () => {
    const { rerender } = render(
      <ReportView detail={reportDetail({ report: null, segments: [] })} onShowLine={vi.fn()} />,
    );
    expect(screen.getByText(/Nobody was heard/)).toBeInTheDocument();
    rerender(<ReportView detail={reportDetail({ report: null })} onShowLine={vi.fn()} />);
    expect(screen.getByText(/no report for this meeting yet/)).toBeInTheDocument();

    const detail = reportDetail({ actionItems: [] });
    if (detail.report) detail.report.openQuestions = [];
    rerender(<ReportView detail={detail} onShowLine={vi.fn()} />);
    expect(screen.queryByRole("region", { name: "Action items" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Open questions" })).toBeNull();
  });
});

describe("labels", () => {
  it("formats costs and finds the line a moment falls in", () => {
    expect(formatCost(0.004)).toBe("under $0.01");
    expect(formatCost(0.256)).toBe("$0.26");
    const lines = [{ startMs: 1_000 }, { startMs: 5_000 }];
    expect(lineAt(lines, 6_000)).toBe(lines[1]);
    expect(lineAt(lines, 5_000)).toBe(lines[1]);
    expect(lineAt(lines, 0)).toBe(lines[0]);
    expect(lineAt<{ startMs: number }>([], 0)).toBeUndefined();
  });

  it("describes the metric behind each part", () => {
    expect(partDetail("llm", {})).toBeNull();
    expect(partDetail("wpm", { my_wpm: 151.6 })).toBe("152 words a minute");
    expect(partDetail("overrun", { overrun_pct: -5 })).toBe("on time");
    expect(partDetail("overrun", { overrun_pct: 12.4 })).toBe("12% over time");
    expect(partDetail("decisions", { decisions_count: 1 })).toBe("1 decision");
    expect(partDetail("interruptions", { interruptions_by_me: 3 })).toBe("3 interruptions");
    expect(partDetail("wpm", {})).toBeNull();
  });
});
