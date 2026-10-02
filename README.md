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

## Quick start (once scaffolded)
```bash
cd app && npm install && npm run tauri dev
```
See AGENTS.md for prerequisites and all commands.
