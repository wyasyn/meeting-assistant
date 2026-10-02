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
  "invalid_state",
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

/** Mirrors Rust `commands::settings::Settings`. FR-8.3 fields join later. */
export interface Settings {
  /** FR-8.5: start on login, minimised to the tray. */
  startOnLogin: boolean;
}

export function getSettings(): Promise<Settings> {
  return call<Settings>("get_settings");
}

/** Applies `settings` and resolves to the settings now in effect. */
export function setSettings(settings: Settings): Promise<Settings> {
  return call<Settings>("set_settings", { settings });
}

/** Mirrors Rust `recording::MeetingSummary`. Times are epoch ms (UTC). */
export interface MeetingSummary {
  id: string;
  title: string;
  sourceApp: string;
  startedAt: number;
  endedAt: number | null;
  durationS: number | null;
  status: string;
}

/** `idle` means no recording since the app started. */
export type RecordingPhase = "idle" | "recording" | "paused" | "stopped";

/** Mirrors Rust `recording::RecordingState`; also the `recording:state` payload. */
export interface RecordingState {
  meetingId: string | null;
  state: RecordingPhase;
  elapsedMs: number;
  /** Why a recording stopped by itself, readable. */
  error: string | null;
}

/** `recording:levels` payload, dBFS from -90 to 0. `null`: no audio from that device. */
export interface RecordingLevels {
  micDb: number | null;
  sysDb: number | null;
}

/** Mirrors Rust `capture::AudioDevice`. */
export interface AudioDevice {
  id: string;
  name: string;
  kind: "input" | "output";
  isDefault: boolean;
}

export function startRecording(args: { title?: string; sourceApp?: string } = {}) {
  return call<MeetingSummary>("start_recording", args);
}

export function pauseRecording() {
  return call<RecordingState>("pause_recording");
}

export function resumeRecording() {
  return call<RecordingState>("resume_recording");
}

export function stopRecording() {
  return call<RecordingState>("stop_recording");
}

export function getRecordingState() {
  return call<RecordingState>("get_recording_state");
}

export function listAudioDevices() {
  return call<AudioDevice[]>("list_audio_devices");
}

export function onRecordingState(handler: (state: RecordingState) => void) {
  return on<RecordingState>("recording:state", handler);
}

export function onRecordingLevels(handler: (levels: RecordingLevels) => void) {
  return on<RecordingLevels>("recording:levels", handler);
}

/** Mirrors Rust `detector::apps::SourceApp`. */
export type DetectedApp = "zoom" | "slack" | "teams" | "discord" | "browser";

/** Mirrors Rust `detector::MeetingDetected`; the `meeting:detected` payload. */
export interface MeetingDetected {
  signalId: string;
  sourceApp: DetectedApp;
  /** Unknown until the extension or the calendar supply it. */
  title: string | null;
  /** 0 to 1. */
  confidence: number;
}

/** `meeting:detected` fires only for apps whose rule is `ask`: it is the consent prompt. */
export function onMeetingDetected(handler: (signal: MeetingDetected) => void) {
  return on<MeetingDetected>("meeting:detected", handler);
}

/** `meeting:prompt-closed` payload: the prompt was answered elsewhere or is no longer needed. */
export interface MeetingPromptClosed {
  signalId: string;
}

export function onMeetingPromptClosed(handler: (closed: MeetingPromptClosed) => void) {
  return on<MeetingPromptClosed>("meeting:prompt-closed", handler);
}

/** Why the recorded meeting seems over (FR-1.6). */
export type EndReason = "app_closed" | "mic_released" | "silence";

/** Mirrors Rust `consent::EndPrompt`; the `meeting:ended` payload. Closed by `meeting:prompt-closed`. */
export interface MeetingEnded {
  signalId: string;
  meetingId: string;
  sourceApp: DetectedApp | null;
  reason: EndReason;
}

/** The recording's meeting seems over: ask whether to stop. Never stops by itself. */
export function onMeetingEnded(handler: (ended: MeetingEnded) => void) {
  return on<MeetingEnded>("meeting:ended", handler);
}

/** Mirrors Rust `store::app_rules::AppRule` (FR-1.7). */
export type AppRule = "ask" | "always" | "never";

/** Mirrors Rust `consent::AppRuleEntry`. */
export interface AppRuleEntry {
  sourceApp: DetectedApp;
  rule: AppRule;
}

/** Every known app with its rule; `ask` when none is set. */
export function listAppRules() {
  return call<AppRuleEntry[]>("list_app_rules");
}

export function setAppRule(sourceApp: DetectedApp, rule: AppRule) {
  return call<undefined>("set_app_rule", { sourceApp, rule });
}

/** Post-call pipeline steps, in order (docs/04 "Pipeline steps"). */
export type JobStep =
  "transcribe_mic" | "transcribe_system" | "merge" | "metrics" | "analyze" | "index" | "notify";

/** Mirrors Rust `jobs::JobProgress`; the `job:progress` payload (NFR-7). */
export interface JobProgress {
  meetingId: string;
  step: JobStep;
  /** `queued` after a failed attempt means a retry is scheduled; `error` says why. */
  status: "queued" | "running" | "done" | "failed";
  attempt: number;
  error?: string;
}

export function onJobProgress(handler: (progress: JobProgress) => void) {
  return on<JobProgress>("job:progress", handler);
}

/** Mirrors Rust `providers::ProviderId` (FR-8.1). */
export type ProviderId = "gemini";

/** Mirrors Rust `providers::ProviderEntry`. The key itself never leaves the keychain. */
export interface ProviderEntry {
  provider: ProviderId;
  hasKey: boolean;
}

/** Mirrors Rust `providers::TestResult`. A refused key is `ok: false`, not an error. */
export interface TestResult {
  ok: boolean;
  message: string;
}

export function listProviders() {
  return call<ProviderEntry[]>("list_providers");
}

/** Saves `key` in the OS keychain. */
export function setApiKey(provider: ProviderId, key: string) {
  return call<undefined>("set_api_key", { provider, key });
}

export function clearApiKey(provider: ProviderId) {
  return call<undefined>("clear_api_key", { provider });
}

/** Asks the provider whether it accepts the saved key. */
export function testProvider(provider: ProviderId) {
  return call<TestResult>("test_provider", { provider });
}

/** Listens to an event and passes only its payload. Resolves to the unlisten function. */
// The per-event helpers pick T, so the single use is intended.
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters
export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => {
    handler(e.payload);
  });
}
