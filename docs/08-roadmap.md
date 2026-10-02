# 08 — Roadmap and task list

Agents: pick the first unchecked task in the current phase, do only that, tick it when "Definition of done" in AGENTS.md is met.

**Current phase: 0**

## Phase 0 — Foundations
- [x] 0.1 Scaffold repo per AGENTS.md layout: Tauri 2 + React + TS (strict) + Vite + Tailwind + shadcn/ui in `app/`; `sidecar/` with uv + FastAPI `/health`; root `.gitignore`, `.editorconfig`, README.
- [x] 0.2 Lint/format/test tooling: ESLint, Prettier, Vitest; rustfmt, clippy (`-D warnings`); ruff, pytest. All commands in AGENTS.md pass on an empty project.
- [ ] 0.3 GitHub Actions: lint + test on Linux; build-only on Windows and macOS.
- [ ] 0.4 `AppError` type, `tracing` setup with file logs, typed `lib/ipc.ts` skeleton.
- [ ] 0.5 Store: SQLCipher connection with key from keychain, migration runner, migration `0001_init.sql` from `05-data-model.md`; repo tests.
- [ ] 0.6 Tray icon with menu (Open, Start recording, Quit) and start-on-login setting (FR-8.5).

## Phase 1 — Record and transcribe (MVP core)
- [ ] 1.1 `AudioBackend` trait + `PipeWireBackend`: list devices, capture mic and default output monitor as two tracks (FR-2.1, FR-2.3). Spike first; record findings in `09-decisions.md`.
- [ ] 1.2 Opus encoding into encrypted 10 s chunks; crash-recovery scan on startup marks interrupted meetings and keeps their audio (FR-2.4, NFR-6).
- [ ] 1.3 Manual recording UI: start/pause/stop, timer, level meters, red indicator in window and tray (FR-1.4, FR-1.5).
- [ ] 1.4 Detector v1: process watch + PipeWire stream watch for Zoom, Slack, Teams, Discord and browsers; debounced `meeting:detected` (FR-1.1, FR-1.2 partial).
- [ ] 1.5 Consent prompt notification with Record / Not now / Never; app rules table (FR-1.3, FR-1.7).
- [ ] 1.6 End detection: app closed / mic released / 2 min silence → prompt to stop (FR-1.6).
- [ ] 1.7 Job queue with retries/backoff and `job:progress` events (NFR-7).
- [ ] 1.8 `Provider` trait + Gemini provider (transcribe + analyze), API key in keychain, settings screen with "Test key" (FR-8.1).
- [ ] 1.9 Pipeline: transcribe mic (is_me) + system (diarized), merge with echo de-dup (FR-3.1, FR-3.2, FR-2.2).
- [ ] 1.10 Transcript view: speakers, timestamps, click-to-play, rename speaker (FR-3.3, FR-3.8).

**Exit criteria:** a real 30-minute Google Meet call on Fedora is detected, recorded with consent, transcribed with "me" correct, speakers renameable, no audio lost when the app is killed mid-call.

## Phase 2 — Summarise and score
- [ ] 2.1 `metrics/` module with table-driven tests (all metrics in `07-scoring.md`).
- [ ] 2.2 Analysis prompt + JSON schema validation + retry on invalid output (FR-4.1, FR-4.2).
- [ ] 2.3 Score combination and storage with evidence (FR-5.1–5.4).
- [ ] 2.4 Report view: summary, decisions, action items, four scores with "why", suggestions (FR-4.3, FR-5.5).
- [ ] 2.5 Meeting library list with scores (FR-6.1).
- [ ] 2.6 Export Markdown + PDF; copy for Slack/email (FR-7.1, FR-7.2).
- [ ] 2.7 Highlight hotkey (FR-9.1); cost display (FR-8.4).

## Phase 3 — Library and habits
- [ ] 3.1 FTS search (FR-6.2, NFR-24).
- [ ] 3.2 Browser extension (Chrome + Firefox) with native messaging (FR-1.2).
- [ ] 3.3 Google Calendar link (FR-1.8, FR-3.4 names).
- [ ] 3.4 Templates + follow-up email (FR-4.4, FR-4.5).
- [ ] 3.5 Trends dashboard (FR-5.6); action items across meetings (FR-6.5).
- [ ] 3.6 Embeddings + Ask (FR-6.4).
- [ ] 3.7 Live captions (FR-3.5).

## Phase 4 — Offline and reach
- [ ] 4.1 Sidecar: faster-whisper + pyannote, spawn/health/idle-stop; local provider (FR-8.2).
- [ ] 4.2 Ollama provider for analysis.
- [ ] 4.3 Flatpak + RPM + AppImage; signed updater (NFR-18, NFR-25).
- [ ] 4.4 Windows WASAPI backend + MSI.
- [ ] 4.5 macOS ScreenCaptureKit backend + DMG.
- [ ] 4.6 Integrations: Notion, Drive, task tools (FR-7.3, FR-7.4).
