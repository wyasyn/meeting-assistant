import { Button } from "@/components/ui/button";
import { APP_NAME } from "@/config";
import { StartOnLoginToggle } from "@/features/settings/StartOnLoginToggle";

function App() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background text-foreground">
      <h1 className="text-2xl font-semibold">{APP_NAME}</h1>
      <p className="text-muted-foreground">Nothing to show yet.</p>
      <Button disabled>Recording arrives in phase 1</Button>
      <StartOnLoginToggle />
    </main>
  );
}

export default App;
