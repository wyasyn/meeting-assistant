import { useEffect, useId, useState } from "react";
import { getSettings, setSettings, toAppError } from "@/lib/ipc";

/** FR-8.5: start on login, minimised to the tray. The OS login item is the source of truth. */
export function StartOnLoginToggle() {
  const id = useId();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getSettings()
      .then((settings) => {
        if (active) setEnabled(settings.startOnLogin);
      })
      .catch((e: unknown) => {
        if (active) setError(toAppError(e).message);
      });
    return () => {
      active = false;
    };
  }, []);

  async function change(next: boolean) {
    setError(null);
    try {
      const settings = await setSettings({ startOnLogin: next });
      setEnabled(settings.startOnLogin);
    } catch (e: unknown) {
      setError(toAppError(e).message);
    }
  }

  return (
    <div className="flex flex-col items-center gap-1">
      <label htmlFor={id} className="flex items-center gap-2 text-sm">
        <input
          id={id}
          type="checkbox"
          className="size-4 accent-primary"
          checked={enabled ?? false}
          disabled={enabled === null}
          onChange={(e) => void change(e.target.checked)}
        />
        Start on login
      </label>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
