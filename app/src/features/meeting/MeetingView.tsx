import { useEffect, useState } from "react";
import { Tabs } from "radix-ui";
import { Button } from "@/components/ui/button";
import { ReportView } from "@/features/report/ReportView";
import { TranscriptView } from "@/features/transcript/TranscriptView";
import { getMeeting, toAppError, type MeetingDetail } from "@/lib/ipc";

type Tab = "report" | "transcript";

const TAB_CLASS =
  "rounded-md px-3 py-1 text-sm font-medium text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none data-[state=active]:bg-accent data-[state=active]:text-foreground";

/** One meeting: the report (FR-4.3) and the transcript (FR-3.1), loaded once. */
export function MeetingView({ meetingId, onBack }: { meetingId: string; onBack: () => void }) {
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("report");
  const [focus, setFocus] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getMeeting(meetingId)
      .then((d) => {
        if (active) setDetail(d);
      })
      .catch((e: unknown) => {
        if (active) setError(toAppError(e).message);
      });
    return () => {
      active = false;
    };
  }, [meetingId]);

  return (
    <section className="flex w-full max-w-2xl flex-col gap-4">
      <div className="flex items-center gap-2">
        <Button type="button" variant="ghost" size="sm" onClick={onBack}>
          Back
        </Button>
        <h2 className="truncate text-lg font-semibold">{detail?.meeting.title}</h2>
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {detail && (
        <Tabs.Root
          value={tab}
          onValueChange={(value) => {
            setTab(value as Tab);
          }}
          className="flex flex-col gap-4"
        >
          <Tabs.List aria-label="Meeting" className="flex gap-1">
            <Tabs.Trigger value="report" className={TAB_CLASS}>
              Report
            </Tabs.Trigger>
            <Tabs.Trigger value="transcript" className={TAB_CLASS}>
              Transcript
            </Tabs.Trigger>
          </Tabs.List>
          <Tabs.Content value="report">
            <ReportView
              detail={detail}
              onShowLine={(segmentId) => {
                setFocus(segmentId);
                setTab("transcript");
              }}
            />
          </Tabs.Content>
          <Tabs.Content value="transcript">
            <TranscriptView
              detail={detail}
              focusSegmentId={focus}
              onChange={(update) => {
                setDetail((d) => d && update(d));
              }}
            />
          </Tabs.Content>
        </Tabs.Root>
      )}
    </section>
  );
}
