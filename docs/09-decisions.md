# 09 — Decisions and open questions

Append-only log. Format: `ADR-NNN — date — title`, then context, decision, consequences (3–6 lines total).

## ADR-001 — 2026-10-02 — Modular monolith desktop app
Single-user, single-machine product. Microservices add hops and failure points with no gain. Decision: one Tauri app with internal modules behind traits + one Python sidecar for local ML. Revisit only for a future sync backend.

## ADR-002 — 2026-10-02 — Tauri 2 + React/TS over Electron and Flutter
Electron fails the idle-RAM target (NFR-1); Flutter desktop is weak for Linux system-audio capture. Rust is limited to core modules; most code stays in TypeScript and Python.

## ADR-003 — 2026-10-02 — Two-track capture; mic = "me"
Separate mic and system tracks make the user's identity certain without diarization and enable echo de-dup. Diarization runs only on the system track.

## ADR-004 — 2026-10-02 — Local-first with bring-your-own key
All data local and encrypted; the user selects a provider and supplies a key (Gemini first). No backend, no telemetry. Offline mode via Whisper + Ollama later.

## ADR-005 — 2026-10-02 — Deterministic metrics + LLM judgement for scores
Metrics computed in code keep scores consistent and explainable; LLM covers qualitative parts and must cite segment IDs.

## ADR-006 — 2026-10-02 — SQLite (SQLCipher, FTS5, sqlite-vec)
One encrypted file, fast local search, no server. UUID v7 + timestamps keep it sync-ready.

## Open questions (owner to decide)
- [ ] Product name and bundle identifier.
- [ ] Default cloud STT: Gemini alone, or Deepgram/AssemblyAI for better diarization?
- [ ] Personal tool only, or team accounts later (affects sync/pricing)?
- [ ] Business model: free with own key, or paid tier with bundled credits?
- [ ] Flatpak sandbox vs process-watching for detection — accept host permissions or rely on extension + PipeWire only?
