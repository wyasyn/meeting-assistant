# 06 — API contracts

Names here are the contract between UI, core, sidecar and providers. Changing one = update this file + `app/src/lib/ipc.ts` + Rust types in the same change. Serde uses `rename_all = "camelCase"`.

## Tauri commands (UI → core)
| Command | Args | Returns | Req |
| --- | --- | --- | --- |
| `start_recording` | `{ meetingId?: string, sourceApp?: string, title?: string }` (`meetingId` reserved for 1.5) | `MeetingSummary` | FR-1.4 |
| `pause_recording` / `resume_recording` / `stop_recording` / `get_recording_state` | none | `RecordingState` | FR-1.4 |
| `respond_to_prompt` | `{ signalId, choice: "record" \| "not_now" \| "never" }` | `void` | FR-1.3 |
| `list_meetings` | `{ query?, tag?, sourceApp?, from?, to?, cursor?, limit? }` | `Page<MeetingSummary>` | FR-6.1 |
| `get_meeting` | `{ id }` | `MeetingDetail` (segments, speakers, report, actions, scores) | FR-4.3 |
| `search` | `{ text, limit? }` | `SearchHit[]` | FR-6.2 |
| `ask` | `{ question, meetingIds? }` | `{ answer, citations: SegmentRef[] }` | FR-6.4 |
| `rename_speaker` | `{ speakerId, name, personId? }` | `Speaker` | FR-3.3 |
| `update_segment_text` | `{ segmentId, text }` | `Segment` | FR-3.7 |
| `set_action_item_done` | `{ id, done }` | `ActionItem` | FR-6.5 |
| `reprocess` | `{ meetingId, fromStep }` | `void` | FR-4.6 |
| `export_meeting` | `{ id, format: "md" \| "pdf" \| "docx" \| "srt" \| "json", path }` | `string` (path) | FR-7.1 |
| `add_highlight` | `{ note? }` | `Highlight` | FR-9.1 |
| `delete_meetings` | `{ ids?: string[], all?: boolean }` | `number` | NFR-15 |
| `get_settings` / `set_settings` | none / `{ settings: Settings }` | `Settings` | FR-8.3, FR-8.5 |
| `set_api_key` / `clear_api_key` | `{ provider, key? }` | `void` (stored in keychain) | FR-8.1 |
| `test_provider` | `{ provider }` | `{ ok, message }` | FR-8.1 |
| `list_audio_devices` | — | `AudioDevice[]` | FR-2.3 |
| `set_app_rule` | `{ sourceApp, rule: "ask" \| "always" \| "never" }` | `void` (`never` also closes that app's open prompt) | FR-1.7 |
| `list_app_rules` | none | `AppRuleEntry[]` | FR-1.7 |

`MeetingSummary { id, title, sourceApp, startedAt, endedAt: number | null, durationS: number | null, status }` (times epoch ms).
`RecordingState { meetingId: string | null, state: "idle" | "recording" | "paused" | "stopped", elapsedMs, error: string | null }`: `idle` = nothing recorded since launch; `elapsedMs` counts recorded time only (paused time excluded); `error` explains a recording that stopped by itself.
`AudioDevice { id, name, kind: "input" | "output", isDefault }` (`id` is the PipeWire `node.name` on Linux). Recording uses the system defaults until the FR-8.3 device setting exists.
`AppRuleEntry { sourceApp, rule }`: one per known app (`zoom`, `slack`, `teams`, `discord`, `browser`), `ask` when none is set. Starting a recording from a prompt is `start_recording { sourceApp }` with no title; the core names it "<App> meeting".

`Settings { startOnLogin: boolean }` for now; FR-8.3 fields (language, summary length, template, retention) join it later. `startOnLogin` is read from the OS login item, not stored in SQLite.

## Tauri events (core → UI)
| Event | Payload |
| --- | --- |
| `meeting:detected` | `{ signalId, sourceApp: "zoom" \| "slack" \| "teams" \| "discord" \| "browser", title: string \| null, confidence }` (0 to 1; `title` null until extension/calendar; never while recording). Sent only when the app's rule is `ask`: it is the consent prompt (FR-1.3); `never` drops the detection and `always` starts recording instead |
| `meeting:prompt-closed` | `{ signalId }`: a consent prompt was answered on the notification, a recording started, or `never` was set for the app; or an end prompt (same `signalId` as its `meeting:ended`) was answered or is moot. The window removes it |
| `meeting:ended` | `{ signalId, meetingId, sourceApp: string \| null, reason: "app_closed" \| "mic_released" \| "silence" }`: the recorded meeting seems over, so the window asks "Meeting over?" (FR-1.6). Never stops the recording by itself; closed with `meeting:prompt-closed { signalId }` when answered on the notification, replaced by a newer end, or the recording stops |
| `recording:state` | `RecordingState` |
| `recording:levels` | `{ micDb, sysDb }` (≤10 Hz), dBFS from -90 to 0; `null` when that device sent no audio in the last 100 ms |
| `caption:segment` | `Segment` (live captions, FR-3.5) |
| `job:progress` | `{ meetingId, step, status, attempt, error? }` |
| `meeting:ready` | `{ meetingId }` |

## Errors
`AppError { code: string, message: string, retryable: boolean }`. Codes: `no_api_key`, `provider_unavailable`, `provider_rejected`, `audio_device`, `permission_denied`, `storage`, `invalid_llm_output`, `not_found`, `sidecar_down`, `invalid_state` (action does not fit the current state, e.g. stop while not recording), `internal` (unexpected failure; generic message, detail in the log file).

## Provider trait (Rust)
```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;            // stt, diarization, llm, embeddings, offline
    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, ProviderError>;
    async fn analyze(&self, req: AnalyzeRequest) -> Result<AnalysisJson, ProviderError>;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError>;
    fn estimate_cost(&self, audio_seconds: u32, transcript_tokens: u32) -> f64;
}
```
`TranscribeRequest { audio_paths, language, diarize: bool, vocabulary }`: `audio_paths` are decrypted temporary Ogg Opus chunk files in time order (chunk format in `05-data-model.md`), deleted after the call → `TranscribeResult { segments: [{start_ms, end_ms, text, speaker_label?, words?}] }`.

## Sidecar HTTP (core → sidecar, 127.0.0.1, `Authorization: Bearer <token>`)
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/health` | — | `{ status, models_loaded }` |
| POST | `/transcribe` | `{ paths: string[], language, word_timestamps: true }` | `{ segments }` |
| POST | `/diarize` | `{ paths: string[], min_speakers?, max_speakers? }` | `{ turns: [{start_ms, end_ms, label}] }` |
| POST | `/embed` | `{ texts: string[] }` | `{ vectors }` |
Paths only; the sidecar decrypts via a key passed at spawn over stdin, never over HTTP.

## Browser extension → core (native messaging, JSON)
`{ "type": "meet:joined" | "meet:left", "platform": "meet" | "zoom_web" | "slack_web", "title"?: string, "url": string, "ts": number }`

## Analysis JSON schema (LLM output — validated before saving)
```json
{
  "type": "object",
  "required": ["summary", "key_points", "decisions", "open_questions", "action_items", "scores", "suggestions"],
  "properties": {
    "summary": { "type": "string", "maxLength": 2000 },
    "key_points": { "type": "array", "items": { "type": "string" } },
    "decisions": { "type": "array", "items": { "type": "object", "required": ["text"], "properties": { "text": {"type": "string"}, "segment_id": {"type": "string"} } } },
    "open_questions": { "type": "array", "items": { "type": "string" } },
    "action_items": { "type": "array", "items": { "type": "object", "required": ["task"], "properties": {
      "task": {"type": "string"}, "owner": {"type": "string"}, "due_date": {"type": "string"}, "segment_id": {"type": "string"} } } },
    "scores": { "type": "object", "required": ["value", "engagement_quality", "my_contribution", "productivity"], "properties": {
      "value": { "$ref": "#/$defs/judged" }, "engagement_quality": { "$ref": "#/$defs/judged" },
      "my_contribution": { "$ref": "#/$defs/judged" }, "productivity": { "$ref": "#/$defs/judged" } } },
    "suggestions": { "type": "array", "minItems": 3, "maxItems": 5, "items": { "type": "object", "required": ["text", "score_kind"], "properties": {
      "text": {"type": "string"}, "score_kind": {"enum": ["engagement", "value", "my_performance", "productivity"]}, "segment_id": {"type": "string"} } } },
    "chapters": { "type": "array", "items": { "type": "object", "properties": { "title": {"type":"string"}, "start_ms": {"type":"integer"} } } }
  },
  "$defs": { "judged": { "type": "object", "required": ["score", "rationale"], "properties": {
    "score": { "type": "integer", "minimum": 0, "maximum": 100 }, "rationale": { "type": "string" },
    "segment_ids": { "type": "array", "items": { "type": "string" } } } } }
}
```
Prompt input: transcript as lines `[segment_id] [mm:ss] Speaker: text`, the deterministic metrics, the template name, and highlights. Any `segment_id` not present in the meeting → reject and retry.
