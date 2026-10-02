# Meeting Assistant (working name)

Desktop app that detects your meetings (Meet, Zoom, Slack, Teams, …), asks to record, and gives you a speaker-labelled transcript, summary, action items, meeting scores and coaching. Local-first; Fedora first, then Windows and macOS.

## For coding agents
Start with `AGENTS.md` (Claude Code also loads it through `CLAUDE.md`). All product context is in `docs/`:

1. `docs/01-vision.md` — vision, users, stakeholders, goals
2. `docs/02-requirements.md` — functional and non-functional requirements
3. `docs/03-tech-stack.md` — stack and rationale
4. `docs/04-architecture.md` — modules and data flow
5. `docs/05-data-model.md` — SQLite schema
6. `docs/06-api-contracts.md` — commands, events, sidecar, provider, LLM schema
7. `docs/07-scoring.md` — how scores work
8. `docs/08-roadmap.md` — phases and task list
9. `docs/09-decisions.md` — decision log and open questions

## Quick start
Install the Fedora prerequisites listed in AGENTS.md first (system packages, Rust stable, Node 20+, uv).

```bash
# Desktop app (Tauri + React)
cd app && npm install && npm run tauri dev

# ML sidecar on its own (the app will spawn it automatically from phase 4)
cd sidecar && uv sync && uv run uvicorn app.main:app --port 8765
curl http://127.0.0.1:8765/health
```

## Layout
- `app/`: Tauri 2 app. React UI in `app/src/`, Rust core in `app/src-tauri/`.
- `sidecar/`: Python FastAPI service for local speech-to-text and diarization.
- `fixtures/audio/`: short synthetic audio for tests (never real meetings).
- `docs/`: product and engineering context.

See AGENTS.md for all lint, test and build commands.
