# AGENTS.md — Meeting Assistant

Working name: **Meeting Assistant** (final name undecided — do not hard-code a brand; use `APP_NAME` from `app/src-tauri/tauri.conf.json` and `app/src/config.ts`).

A cross-platform desktop app that detects when the user joins a call (Google Meet, Zoom, Slack, Teams, Discord, etc.), asks permission to record, then produces a speaker-labelled transcript, summary, action items, four meeting scores and coaching tips. **Fedora (Wayland, PipeWire) is the primary target**; Windows and macOS follow from the same codebase.

Read before coding:
| Need | File |
| --- | --- |
| Why we build it, users, stakeholders, principles | `docs/01-vision.md` |
| What it must do (FR/NFR with IDs) | `docs/02-requirements.md` |
| Stack and why | `docs/03-tech-stack.md` |
| Modules, processes, data flow | `docs/04-architecture.md` |
| SQLite schema (source of truth) | `docs/05-data-model.md` |
| Tauri commands/events, sidecar API, provider trait, analysis JSON | `docs/06-api-contracts.md` |
| How scores are calculated | `docs/07-scoring.md` |
| Phases and the task list | `docs/08-roadmap.md` |
| Decisions made + open questions | `docs/09-decisions.md` |

Reference requirement IDs (e.g. `FR-2.1`, `NFR-11`) in commit messages, PR descriptions and test names.

## Stack (summary)
- Shell: **Tauri 2**, Rust core (stable toolchain, edition 2021).
- UI: **React 19 + TypeScript (strict) + Vite**, Tailwind CSS v4, shadcn/ui, TanStack Query, Zustand for UI state.
- Audio: `pipewire` crate (Linux), `cpal` WASAPI loopback (Windows), ScreenCaptureKit (macOS, later). Opus encoding.
- DB: **SQLite** via `rusqlite` with SQLCipher, FTS5, sqlite-vec. Migrations in `app/src-tauri/migrations/`.
- Sidecar: **Python 3.11+, FastAPI**, faster-whisper, pyannote.audio; managed with `uv`.
- AI: Gemini API (user's own key) behind a `Provider` trait; Deepgram/AssemblyAI optional; Ollama for offline.
- Secrets: OS keychain via `keyring` crate. Never in SQLite, files, logs or env files committed.

## Repository layout
```
app/                      Tauri app
  src/                    React UI (routes/, components/, features/<feature>/, lib/ipc.ts)
  src-tauri/
    src/
      main.rs             wiring only
      commands/           #[tauri::command] handlers — thin, call services
      detector/           meeting detection (process, PipeWire streams, extension msgs, calendar)
      capture/            audio capture per OS behind `AudioBackend` trait
      jobs/               persistent job queue + pipeline steps
      metrics/            deterministic metrics (talk ratio, wpm, fillers, interruptions)
      providers/          `Provider` trait + gemini/, deepgram/, local/ impls
      store/              SQLite repo layer, migrations, encryption
      sidecar/            spawn/health/stop the Python process
      error.rs            AppError (thiserror) → serialisable to UI
    migrations/           NNNN_description.sql
sidecar/                  FastAPI service (app/main.py, routers/, services/, tests/)
extension/                browser extension (Phase 3)
docs/                     context docs listed above
```
If the repo does not exist yet, scaffold it exactly like this (Phase 0 in the roadmap).

## Commands
Fedora prerequisites (once):
```bash
sudo dnf group install "c-development"
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel pipewire-devel clang opus-devel
curl -LsSf https://astral.sh/uv/install.sh | sh
```
Daily:
```bash
cd app && npm install && npm run tauri dev          # run the app
cd app && npm run lint && npm run typecheck && npm test
cd app && npm run format                             # prettier --write
cd app && npm run test:coverage                      # vitest with v8 coverage
cd app/src-tauri && cargo fmt --check && cargo clippy -- -D warnings && cargo test
cd sidecar && uv sync && uv run ruff check . && uv run ruff format --check . && uv run pytest
cd sidecar && uv run uvicorn app.main:app --port 0   # sidecar alone (core normally spawns it)
```
Keep these commands working. If you add a script, add it here.

## Rules that must not be broken
1. **Never record silently.** Recording starts only from a user action or a per-app rule the user set (FR-1.3, FR-1.7, NFR-12). The tray/indicator must reflect recording state at all times.
2. **Local-first.** Audio, transcripts and reports stay on disk. The only outbound traffic is to the AI provider the user selected, over TLS (NFR-11, NFR-14). No telemetry, no analytics SDKs.
3. **Never lose audio.** Write audio in ~10 s Opus chunks as it is captured; a crash loses at most one chunk (NFR-6). Delete audio only after retention rules allow it.
4. **"Me" comes from the mic track.** Do not infer the user's identity by diarization; mic-track segments are `is_me = true` (FR-2.2).
5. **Scores are explainable.** Every score stores its evidence (metric values and/or segment IDs). Deterministic metrics are computed in Rust (`metrics/`), not by the LLM (`docs/07-scoring.md`).
6. **LLM output is validated.** Analysis responses must match the JSON schema in `docs/06-api-contracts.md`; on failure the job retries, it never saves partial/invalid data.
7. **Providers are swappable.** UI and jobs depend on the `Provider` trait only — never import a vendor SDK outside `providers/<vendor>/`.
8. **Secrets** go through `keyring`. Never log API keys, transcripts, or audio paths at `info` level or above.
9. **Schema changes** = new numbered migration + update `docs/05-data-model.md` in the same change. Never edit an applied migration.
10. **Cross-platform.** OS-specific code lives behind traits with `#[cfg(target_os = ...)]` impls. Fedora/Wayland must keep working; do not rely on X11-only APIs (e.g. reading other windows' titles).

## Code conventions
**Rust:** `thiserror` for errors, `anyhow` only in tests/bin; no `unwrap()`/`expect()` outside tests; `tokio` for async; `tracing` for logs; services are structs with injected dependencies (traits) so they can be unit-tested with fakes. Commands return `Result<T, AppError>`.
**TypeScript:** strict mode, no `any`; all IPC goes through typed wrappers in `app/src/lib/ipc.ts` (one function per command, one listener helper per event, types mirror Rust structs — keep names identical, camelCase via serde `rename_all = "camelCase"`). Feature folders under `src/features/`. Components are function components; server state in TanStack Query, UI state in Zustand.
**Python:** type hints everywhere, Pydantic models for every request/response, `ruff` formatting, no global model loading at import time (load lazily, keep warm).
**IDs:** UUID v7 strings everywhere. **Time:** store UTC ISO-8601 / epoch ms; display in local time.
**UI copy:** sentence case, plain words; recording state always uses the same red dot + "Recording" label.

## Testing
- Rust: unit tests beside code; `metrics/` must have table-driven tests with hand-built segment fixtures. Pipeline steps tested with fake `Provider` and fake `AudioBackend`.
- Sample audio fixtures in `fixtures/audio/` (short, synthetic or permissively licensed; never real meetings).
- TS: Vitest + Testing Library for components with logic; mock `ipc.ts`.
- Python: pytest with a tiny audio fixture; mark slow model tests `@pytest.mark.slow`.
- Target ≥70% coverage on `metrics/`, `jobs/`, `store/`, `providers/` (NFR-23).

## Definition of done (every task)
- [ ] Requirement IDs addressed are named in the commit/PR.
- [ ] Lint, typecheck, format and all tests pass (commands above).
- [ ] New behaviour has tests; failure paths handled and surfaced to the UI as a readable message.
- [ ] Docs updated if contracts, schema or decisions changed.
- [ ] Manually run on Fedora when the change touches capture, detection, tray or notifications.

## Working style for agents
- Work one roadmap task at a time; keep diffs small and focused.
- Prefer boring, well-maintained crates/packages; ask before adding any dependency.
- When requirements are unclear or conflict, stop and ask — list the options and your recommendation. Do not invent product behaviour.
- Do not commit generated binaries, model weights, `.env` files, or recordings.
- Conventional commits: `feat(capture): two-track pipewire recorder (FR-2.1, FR-2.2)`.
