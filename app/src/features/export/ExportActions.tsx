import { useState } from "react";
import { Button } from "@/components/ui/button";
import { AppError, exportMeeting, pickSavePath, toAppError, type MeetingDetail } from "@/lib/ipc";
import { fileName, toEmail, toMarkdown, toSlack } from "./format";

/**
 * FR-7.1, FR-7.2: save as Markdown, print to PDF, copy for Slack or email. `onPrint` shows
 * the report and opens the system print dialog, where "Print to file" makes the PDF.
 */
export function ExportActions({ detail, onPrint }: { detail: MeetingDetail; onPrint: () => void }) {
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(work: () => Promise<string | null>) {
    setStatus(null);
    setError(null);
    try {
      setStatus(await work());
    } catch (e: unknown) {
      setError(toAppError(e).message);
    }
  }

  async function saveMarkdown() {
    const path = await pickSavePath(fileName(detail, "md"), {
      name: "Markdown",
      extensions: ["md"],
    });
    if (path === null) return null;
    const written = await exportMeeting(detail.meeting.id, "md", path, toMarkdown(detail));
    return `Saved to ${written}`;
  }

  async function copy(text: string, done: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      throw new AppError({
        code: "internal",
        message: "Could not copy to the clipboard.",
        retryable: false,
      });
    }
    return done;
  }

  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" onClick={() => void run(saveMarkdown)}>
          Export Markdown
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onPrint}>
          Export PDF
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void run(() => copy(toSlack(detail), "Copied for Slack."))}
        >
          Copy for Slack
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void run(() => copy(toEmail(detail), "Copied for email."))}
        >
          Copy for email
        </Button>
      </div>
      <p role="status" className="text-sm text-muted-foreground">
        {status}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
