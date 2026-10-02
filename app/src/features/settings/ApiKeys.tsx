import { useEffect, useId, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  clearApiKey,
  listProviders,
  setApiKey,
  testProvider,
  toAppError,
  type ProviderEntry,
  type ProviderId,
  type TestResult,
} from "@/lib/ipc";

const PROVIDERS: Record<ProviderId, { label: string; keyFrom: string }> = {
  gemini: { label: "Gemini", keyFrom: "Google AI Studio" },
};

/** FR-8.1: bring your own API key, kept in the system keychain. */
export function ApiKeys() {
  const id = useId();
  const [providers, setProviders] = useState<ProviderEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    listProviders()
      .then((list) => {
        if (active) setProviders(list);
      })
      .catch((e: unknown) => {
        if (active) setError(toAppError(e).message);
      });
    return () => {
      active = false;
    };
  }, []);

  function setHasKey(index: number, hasKey: boolean) {
    setProviders((list) => (list ?? []).map((e, i) => (i === index ? { ...e, hasKey } : e)));
  }

  return (
    <section aria-labelledby={id} className="flex w-full max-w-sm flex-col gap-2">
      <h2 id={id} className="text-sm font-medium">
        AI provider
      </h2>
      {providers?.map((entry, index) => (
        <ProviderKey
          key={entry.provider}
          entry={entry}
          onChange={(hasKey) => {
            setHasKey(index, hasKey);
          }}
        />
      ))}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}

function ProviderKey({
  entry,
  onChange,
}: {
  entry: ProviderEntry;
  onChange: (hasKey: boolean) => void;
}) {
  const inputId = useId();
  const { label, keyFrom } = PROVIDERS[entry.provider];
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<TestResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      await action();
    } catch (e: unknown) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  const save = () =>
    run(async () => {
      await setApiKey(entry.provider, draft);
      setDraft("");
      onChange(true);
    });

  const test = () =>
    run(async () => {
      setResult(await testProvider(entry.provider));
    });

  const remove = () =>
    run(async () => {
      await clearApiKey(entry.provider);
      onChange(false);
    });

  return (
    <div className="flex flex-col gap-2 text-sm">
      <div className="flex items-center justify-between gap-4">
        <label htmlFor={inputId}>{label} API key</label>
        <span className="text-muted-foreground">{entry.hasKey ? "Key saved" : "No key saved"}</span>
      </div>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <input
          id={inputId}
          type="password"
          autoComplete="off"
          spellCheck={false}
          className="min-w-0 flex-1 rounded-md border bg-background px-2 py-1"
          placeholder={entry.hasKey ? "Paste a new key to replace it" : "Paste your key"}
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
          }}
        />
        <Button type="submit" size="sm" disabled={busy || draft.trim() === ""}>
          Save
        </Button>
      </form>
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={busy || !entry.hasKey}
          onClick={() => void test()}
        >
          {busy ? "Working..." : "Test key"}
        </Button>
        {entry.hasKey && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => void remove()}
          >
            Remove key
          </Button>
        )}
      </div>
      <p className="text-muted-foreground">
        Get a key from {keyFrom}. It is stored in your system keychain and only sent to {label}.
      </p>
      {result && (
        <p role="status" className={result.ok ? "text-foreground" : "text-destructive"}>
          {result.message}
        </p>
      )}
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
