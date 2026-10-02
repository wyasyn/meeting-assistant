import { useEffect, useId, useState } from "react";
import { APP_LABELS } from "@/features/consent/apps";
import { listAppRules, setAppRule, toAppError, type AppRule, type AppRuleEntry } from "@/lib/ipc";

const RULE_LABELS: Record<AppRule, string> = {
  ask: "Always ask",
  always: "Always record",
  never: "Never record",
};

const RULES: AppRule[] = ["ask", "always", "never"];

function isAppRule(value: string): value is AppRule {
  return (RULES as string[]).includes(value);
}

/** FR-1.7: what to do when each meeting app is detected. */
export function AppRules() {
  const id = useId();
  const [rules, setRules] = useState<AppRuleEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    listAppRules()
      .then((list) => {
        if (active) setRules(list);
      })
      .catch((e: unknown) => {
        if (active) setError(toAppError(e).message);
      });
    return () => {
      active = false;
    };
  }, []);

  async function change(entry: AppRuleEntry, rule: AppRule) {
    setError(null);
    try {
      await setAppRule(entry.sourceApp, rule);
      setRules((list) =>
        (list ?? []).map((e) => (e.sourceApp === entry.sourceApp ? { ...e, rule } : e)),
      );
    } catch (e: unknown) {
      setError(toAppError(e).message);
    }
  }

  return (
    <section aria-labelledby={id} className="flex w-full max-w-sm flex-col gap-2">
      <h2 id={id} className="text-sm font-medium">
        When a meeting starts
      </h2>
      {rules?.map((entry) => (
        <label key={entry.sourceApp} className="flex items-center justify-between gap-4 text-sm">
          {APP_LABELS[entry.sourceApp]}
          <select
            className="rounded-md border bg-background px-2 py-1"
            value={entry.rule}
            onChange={(e) => {
              const rule = e.target.value;
              if (isAppRule(rule)) void change(entry, rule);
            }}
          >
            {RULES.map((rule) => (
              <option key={rule} value={rule}>
                {RULE_LABELS[rule]}
              </option>
            ))}
          </select>
        </label>
      ))}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
