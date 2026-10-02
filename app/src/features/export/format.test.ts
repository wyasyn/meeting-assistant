import { describe, expect, it } from "vitest";
import { reportDetail } from "@/features/report/fixtures";
import { formatDate } from "@/features/report/labels";
import { fileName, toEmail, toMarkdown, toSlack } from "./format";

const when = new Date(Date.UTC(2026, 9, 2, 9, 0)).toLocaleString(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});
const due = formatDate("2026-10-09");

describe("toMarkdown (FR-7.1)", () => {
  it("holds the report, the scores and the transcript", () => {
    const md = toMarkdown(reportDetail());
    for (const part of [
      "# Planning\n",
      `- When: ${when}\n- Length: 30:00\n- Participants: Me, Ann\n`,
      "## Summary\n\nPlanned the release.\nAgreed on Friday.\n",
      "- Engagement: 72/100. Both people asked and answered questions.\n- Value of discussion: 54/100\n",
      "## Decisions\n\n- Ship on Friday (at 2:05)\n",
      `- [ ] Write release notes (You, due ${due}) (at 2:05)\n- [ ] Book the demo room (Ann)\n`,
      "## Open questions\n\n- Who writes the notes?\n",
      "- Ask Ann for her view earlier (Engagement) (at 0:01)\n",
      "## Chapters\n\n- 0:00 Release date\n",
      "## Transcript\n\n**0:01 Ann:** Can we ship on Friday?\n\n**2:05 Me:** Friday works.\n",
    ]) {
      expect(md).toContain(part);
    }
  });

  it("says when there was too little data, and works without a report", () => {
    expect(toMarkdown(reportDetail({ scores: [] }))).toContain(
      "## Scores\n\nNot enough data to score.\n",
    );
    const bare = toMarkdown(reportDetail({ report: null, actionItems: [] }));
    expect(bare).not.toContain("## Summary");
    expect(bare).toContain("## Transcript");
  });
});

describe("toSlack and toEmail (FR-7.2)", () => {
  it("share the outcomes without scores or coaching", () => {
    const slack = toSlack(reportDetail());
    expect(slack).toBe(
      `*Planning* (${when})\n\n` +
        "*Summary*\nPlanned the release.\nAgreed on Friday.\n\n" +
        "*Decisions*\n• Ship on Friday\n\n" +
        `*Action items*\n• Write release notes (Me, due ${due})\n• Book the demo room (Ann)\n\n` +
        "*Open questions*\n• Who writes the notes?\n",
    );
    const email = toEmail(reportDetail());
    expect(email).toBe(
      `Planning, ${when}\n\n` +
        "Summary\nPlanned the release.\nAgreed on Friday.\n\n" +
        "Decisions\n- Ship on Friday\n\n" +
        `Action items\n- Write release notes (Me, due ${due})\n- Book the demo room (Ann)\n\n` +
        "Open questions\n- Who writes the notes?\n",
    );
    for (const text of [slack, email]) {
      expect(text).not.toMatch(/Engagement|72|Suggestion/);
    }
  });

  it("leave out empty lists", () => {
    const detail = reportDetail({ actionItems: [] });
    if (detail.report) detail.report.openQuestions = [];
    expect(toSlack(detail)).not.toMatch(/Action items|Open questions/);
  });
});

describe("fileName", () => {
  it("drops characters file systems refuse", () => {
    const detail = reportDetail();
    expect(fileName(detail, "md")).toBe("Planning.md");
    detail.meeting.title = 'Q3: plan/review "draft"';
    expect(fileName(detail, "md")).toBe("Q3  plan review  draft.md");
    detail.meeting.title = "///";
    expect(fileName(detail, "md")).toBe("Meeting.md");
  });
});
