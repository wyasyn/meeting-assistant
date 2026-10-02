import { useEffect, useState } from "react";
import { APP_NAME } from "@/config";
import { ConsentPrompts } from "@/features/consent/ConsentPrompts";
import { EndPrompt } from "@/features/consent/EndPrompt";
import { initConsent } from "@/features/consent/store";
import { MeetingView } from "@/features/meeting/MeetingView";
import { RecordingControls } from "@/features/recording/RecordingControls";
import { RecordingIndicator } from "@/features/recording/RecordingIndicator";
import { initRecording } from "@/features/recording/store";
import { ApiKeys } from "@/features/settings/ApiKeys";
import { AppRules } from "@/features/settings/AppRules";
import { StartOnLoginToggle } from "@/features/settings/StartOnLoginToggle";
import { MeetingList } from "@/features/library/MeetingList";

function App() {
  useEffect(() => initRecording(), []);
  useEffect(() => initConsent(), []);
  const [openMeeting, setOpenMeeting] = useState<string | null>(null);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-background px-4 py-12 text-foreground">
      {/* Printing a report (PDF export) shows the meeting only. */}
      <div className="contents print:hidden">
        <RecordingIndicator />
        <h1 className="text-2xl font-semibold">{APP_NAME}</h1>
        <ConsentPrompts />
        <EndPrompt />
        <RecordingControls />
      </div>
      {openMeeting ? (
        <MeetingView
          meetingId={openMeeting}
          onBack={() => {
            setOpenMeeting(null);
          }}
        />
      ) : (
        <>
          <MeetingList onOpen={setOpenMeeting} />
          <AppRules />
          <ApiKeys />
          <StartOnLoginToggle />
        </>
      )}
    </main>
  );
}

export default App;
