import { useEffect } from "react";
import { APP_NAME } from "@/config";
import { RecordingControls } from "@/features/recording/RecordingControls";
import { RecordingIndicator } from "@/features/recording/RecordingIndicator";
import { initRecording } from "@/features/recording/store";
import { StartOnLoginToggle } from "@/features/settings/StartOnLoginToggle";

function App() {
  useEffect(() => initRecording(), []);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-background px-4 text-foreground">
      <RecordingIndicator />
      <h1 className="text-2xl font-semibold">{APP_NAME}</h1>
      <RecordingControls />
      <StartOnLoginToggle />
    </main>
  );
}

export default App;
