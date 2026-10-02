# 06 — API contracts

Names here are the contract between UI, core, sidecar and providers. Changing one = update this file + `app/src/lib/ipc.ts` + Rust types in the same change. Serde uses `rename_all = "camelCase"`.

## Tauri commands (UI → core)
| Command | Args | Returns | Req |
| --- | --- | --- | --- |
| `start_recording` | `{ meetingId?: string, sourceApp?: string, title?: string }` (`meetingId` reserved for 1.5) | `MeetingSummary` | FR-1.4 |
| `pause_recording` / `resume_recording` / `stop_recording` / `get_recording_state` | none | `RecordingState` | FR-1.4 |
| `respond_to_prompt` | `{ signalId, choice: "record" \| "not_now" \| "never" }` | `void` | FR-1.3 |
| `list_meetings` | `{ query?, tag?, sourceApp?, from?, to?, cursor?, limit? }` (only `cursor` and `limit` so far; `limit` 1 to 200, default 50) | `Page<MeetingListItem>`, newest first | FR-6.1 |
| `get_meeting` | `{ id }` | `MeetingDetail` (segments, speakers, report, actions, scores) | FR-4.3 |
| `search` | `{ text, limit? }` | `SearchHit[]` | FR-6.2 |
| `ask` | `{ question, meetingIds? }` | `{ answer, citations: SegmentRef[] }` | FR-6.4 |
| `rename_speaker` | `{ speakerId, name, personId? }` (`personId` reserved for FR-3.4) | `Speaker`: the one that now holds the name. A name another speaker on the same side already has (ignoring case) merges the two; blank or over 100 characters is `invalid_state` | FR-3.3 |
| `get_segment_audio` | `{ segmentId }` | raw bytes (`ArrayBuffer`): the segment's stretch of its own track as a 16 kHz mono 16-bit WAV, at most 10 min; `not_found` once the audio is deleted (ADR-022) | FR-3.8 |
| `update_segment_text` | `{ segmentId, text }` | `Segment` | FR-3.7 |
| `set_action_item_done` | `{ id, done }` | `ActionItem` | FR-6.5 |
| `reprocess` | `{ meetingId, fromStep }` | `void` | FR-4.6 |
| `export_meeting` | `{ id, format: "md" \| "pdf" \| "docx" \| "srt" \| "json", path }` | `string` (path) | FR-7.1 |
| `add_highlight` | `{ note? }` | `Highlight` | FR-9.1 |
| `delete_meetings` | `{ ids?: string[], all?: boolean }` | `number` | NFR-15 |
| `get_settings` / `set_settings` | none / `{ settings: Settings }` | `Settings` | FR-8.3, FR-8.5 |
| `list_providers` | none | `ProviderEntry[]` | FR-8.1 |
| `set_api_key` / `clear_api_key` | `{ provider, key }` / `{ provider }` | `void` (stored in / removed from the keychain; the key is trimmed, blank is `invalid_state`; saving also resumes meetings waiting for a key) | FR-8.1 |
| `test_provider` | `{ provider }` | `TestResult` (a refused or unreachable key is `ok: false`; only keychain failures are errors) | FR-8.1 |
| `list_audio_devices` | — | `AudioDevice[]` | FR-2.3 |
| `set_app_rule` | `{ sourceApp, rule: "ask" \| "always" \| "never" }` | `void` (`never` also closes that app's open prompt) | FR-1.7 |
| `list_app_rules` | none | `AppRuleEntry[]` | FR-1.7 |

`MeetingSummary { id, title, sourceApp, startedAt, endedAt: number | null, durationS: number | null, status }` (times epoch ms).
`MeetingListItem`: the `MeetingSummary` fields plus `participants: string[]` (labels of the other side in order of first appearance, "Me" left out) and `scores: { kind, value }[]` (headline scores in display order, empty until scored or with too little data).
`Page<T> { items: T[], nextCursor: string | null }`: pass `nextCursor` back as `cursor`; `null` on the last page.
`MeetingDetail { meeting: MeetingSummary, speakers: Speaker[], segments: Segment[], hasAudio, report: Report | null, actionItems: ActionItem[], scores: Score[] }`: speakers "Me" first; segments of both tracks in time order; `hasAudio` false once retention deleted the audio; `report` null until analyzed or when nobody was heard; `scores` holds the headline scores only, in the order engagement, value, my_performance, productivity, and is empty when there was too little data to score (ADR-025).
`Report { summary, keyPoints: string[], decisions: { text, segmentId }[], openQuestions: string[], suggestions: { text, scoreKind, segmentId }[], chapters: { title, startMs }[], modelUsed, costUsd: number | null, createdAt }`. `ActionItem { id, task, ownerLabel: string | null, dueDate: string | null, done, segmentId: string | null }` (`ownerLabel` "Me" is the user; `dueDate` YYYY-MM-DD). `Score { kind, value, evidence: { metrics: Record<string, number>, parts: { name, weight, score }[], segmentIds: string[], rationale } }`. Every `segmentId` is a segment of the meeting.
`Speaker { id, label, isMe }`. `Segment { id, track: "mic" | "sys", speakerId: string | null, startMs, endMs, text }` (ms from the meeting start; `speakerId` null when the provider could not tell who spoke).
`RecordingState { meetingId: string | null, state: "idle" | "recording" | "paused" | "stopped", elapsedMs, error: string | null }`: `idle` = nothing recorded since launch; `elapsedMs` counts recorded time only (paused time excluded); `error` explains a recording that stopped by itself.
`AudioDevice { id, name, kind: "input" | "output", isDefault }` (`id` is the PipeWire `node.name` on Linux). Recording uses the system defaults until the FR-8.3 device setting exists.
`AppRuleEntry { sourceApp, rule }`: one per known app (`zoom`, `slack`, `teams`, `discord`, `browser`), `ask` when none is set. Starting a recording from a prompt is `start_recording { sourceApp }` with no title; the core names it "<App> meeting".

`ProviderEntry { provider: "gemini", hasKey }`: one per provider; the key itself is never sent to the UI. `TestResult { ok, message }`: `message` is readable either way.

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
| `job:progress` | `{ meetingId, step, status: "queued" \| "running" \| "done" \| "failed", attempt, error? }`: `running` when an attempt starts (attempt from 1), `done` when it succeeds, `queued` with `error` when a retry is scheduled or, for a missing API key, when the step waits until a key is saved (`set_api_key` resumes it, ADR-021), `failed` with `error` when the step gave up and the meeting is `failed`. `error` is readable text, absent when there is none (NFR-7) |
| `meeting:ready` | `{ meetingId }` |

## Errors
`AppError { code: string, message: string, retryable: boolean }`. Codes: `no_api_key`, `provider_unavailable`, `provider_rejected`, `audio_device`, `permission_denied`, `storage`, `invalid_llm_output`, `not_found`, `sidecar_down`, `invalid_state` (action does not fit the current state, e.g. stop while not recording), `internal` (unexpected failure; generic message, detail in the log file).

## Provider trait (Rust)
```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;            // stt, diarization, llm, embeddings, offline
    async fn check(&self) -> Result<(), ProviderError>; // "Test key": a free call that proves the key works
    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, ProviderError>;
    async fn analyze(&self, req: AnalyzeRequest) -> Result<AnalysisJson, ProviderError>;
    fn analysis_model(&self) -> String;                // stored in reports.model_used
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError>;
    fn estimate_cost(&self, audio_seconds: u32, transcript_tokens: u32) -> f64;
}
```
`TranscribeRequest { chunks: [{ ogg, start_ms }], language?, diarize: bool, vocabulary }`: `chunks` are decrypted Ogg Opus chunks held in memory only, never written to disk (ADR-021), in time order with their start in the meeting (chunk format in `05-data-model.md`) → `TranscribeResult { segments: [{start_ms, end_ms, text, speaker_label?}] }`, times in ms from the meeting start. A `speaker_label` never stands for two voices; one voice may get two labels when a long meeting is sent in batches. Word timings are not returned yet.
`ProviderError`: `Unavailable` → `provider_unavailable` (retryable), `Rejected` and `Unsupported` → `provider_rejected`, `InvalidOutput` → `invalid_llm_output` (retryable). Messages are readable and never hold keys, transcript text or paths.
Gemini (ADR-020): `gemini-3.5-flash-lite` transcribes, `gemini-3.8-flash` analyzes; REST `generateContent` with the key in the `x-goog-api-key` header, JSON output constrained by `responseSchema`. `embed` is `Unsupported` for now.

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
Prompt input (`providers/prompt.rs`, shared by every LLM provider): transcript as lines `[ref] [mm:ss] Speaker: text`, the deterministic metrics (talk shares by speaker name), the template name, and highlights. `ref` is a short stand-in for the segment id (`s1`, `s2`, ... in time order) that the analyze step maps back before saving, so every `segment_id`/`segment_ids` in the saved report is a real segment id. Any ref not present in the meeting → reject the whole answer and retry (`invalid_llm_output`, ADR-024). The user is always "Me" in the prompt, whatever their speaker is named.
