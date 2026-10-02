// Typed wrappers for every Tauri command and event (AGENTS.md, docs/06-api-contracts.md).
// Add one function per command and one listener helper per event, built on `call` and `on`.
// Components never import `@tauri-apps/api` directly; tests mock this module.
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const APP_ERROR_CODES = [
  "no_api_key",
  "provider_unavailable",
  "provider_rejected",
  "audio_device",
  "permission_denied",
  "storage",
  "invalid_llm_output",
  "not_found",
  "sidecar_down",
  "internal",
] as const;

export type AppErrorCode = (typeof APP_ERROR_CODES)[number];

/** Wire shape of Rust `AppError`. */
export interface AppErrorPayload {
  code: AppErrorCode;
  message: string;
  retryable: boolean;
}

/** Same text as `INTERNAL_MESSAGE` in `src-tauri/src/error.rs`. */
export const INTERNAL_MESSAGE = "Something went wrong. Details are in the log file.";

/** Every failed `call` rejects with this, so the UI can always show `message`. */
export class AppError extends Error {
  readonly code: AppErrorCode;
  readonly retryable: boolean;
  /** The original rejection when it was not an `AppErrorPayload`. */
  readonly original: unknown;

  constructor(payload: AppErrorPayload, original?: unknown) {
    super(payload.message);
    this.name = "AppError";
    this.code = payload.code;
    this.retryable = payload.retryable;
    this.original = original;
  }
}

export function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  if (typeof value !== "object" || value === null) return false;
  const { code, message, retryable } = value as Record<string, unknown>;
  return (
    typeof code === "string" &&
    (APP_ERROR_CODES as readonly string[]).includes(code) &&
    typeof message === "string" &&
    typeof retryable === "boolean"
  );
}

export function toAppError(error: unknown): AppError {
  if (error instanceof AppError) return error;
  if (isAppErrorPayload(error)) return new AppError(error);
  return new AppError({ code: "internal", message: INTERNAL_MESSAGE, retryable: false }, error);
}

/** Invokes a command. Rejects with `AppError` only. */
export async function call<T>(command: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throw toAppError(error);
  }
}

/** Listens to an event and passes only its payload. Resolves to the unlisten function. */
// The per-event helpers pick T, so the single use is intended.
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters
export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => {
    handler(e.payload);
  });
}
