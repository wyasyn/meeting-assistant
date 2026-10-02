import { useEffect } from "react";
import { APP_NAME } from "@/config";
import { ConsentPrompts } from "@/features/consent/ConsentPrompts";
import { EndPrompt } from "@/features/consent/EndPrompt";
import { initConsent } from "@/features/consent/store";
import { RecordingControls } from "@/features/recording/RecordingControls";
import { RecordingIndicator } from "@/features/recording/RecordingIndicator";
import { initRecording } from "@/features/recording/store";
import { ApiKeys } from "@/features/settings/ApiKeys";
import { AppRules } from "@/features/settings/AppRules";
import { StartOnLoginToggle } from "@/features/settings/StartOnLoginToggle";

function App() {
  useEffect(() => initRecording(), []);
  useEffect(() => initConsent(), []);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-background px-4 text-foreground">
      <RecordingIndicator />
      <h1 className="text-2xl font-semibold">{APP_NAME}</h1>
      <ConsentPrompts />
      <EndPrompt />
      <RecordingControls />
      <AppRules />
      <ApiKeys />
      <StartOnLoginToggle />
    </main>
  );
}

export default App;
