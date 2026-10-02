# 03 — Tech stack

Performance target is one machine: low CPU while recording, fast post-call processing, a library that stays fast at 1,000+ meetings. No server is needed for the MVP.

| Layer | Choice | Why |
| --- | --- | --- |
| Desktop shell | Tauri 2 (Rust core) | ~10× lighter than Electron; RPM/Flatpak/AppImage, MSI, DMG from one codebase |
| UI | React 19 + TypeScript (strict) + Vite, Tailwind v4, shadcn/ui, TanStack Query, Zustand | Owner's strongest stack; runs in Tauri webview |
| Audio capture | `pipewire` crate (Linux), `cpal` WASAPI loopback (Windows), ScreenCaptureKit (macOS) | Two separate tracks, low CPU |
| Audio format | Opus (`audiopus`/`opus` crate), 16 kHz mono per track, 10 s chunks | ~1 MB / 10 min; crash-safe |
| Meeting detection | `sysinfo` (processes), PipeWire stream events (mic in use), browser extension via native messaging, Google Calendar API | Wayland blocks reading other windows' titles |
| Database | SQLite (`rusqlite` + `bundled-sqlcipher`), FTS5, sqlite-vec | One encrypted file, ms search |
| Secrets | `keyring` crate (libsecret / Keychain / Credential Manager) | Keys never in plain text |
| Job queue | Own table-backed queue in SQLite + `tokio` workers | Resumable, no extra infra |
| ML sidecar | Python 3.11+, FastAPI, faster-whisper, pyannote.audio, packaged with PyInstaller; deps via `uv` | Offline STT + diarization; owner knows FastAPI |
| Cloud AI | Gemini API (user's key) for analysis; Deepgram / AssemblyAI optional STT with diarization | Cheap long-context analysis; strong diarization |
| Local AI | Ollama (Llama / Qwen family) | Offline analysis |
| Logging | `tracing` + rolling file appender | Debuggable without leaking content |
| Testing | cargo test, Vitest + Testing Library, pytest | |
| CI | GitHub Actions: Linux (Fedora container), Windows, macOS builds | NFR-23 |
| Packaging | Tauri bundler (RPM, AppImage), Flatpak manifest; later MSI/DMG; signed updater | NFR-18, NFR-25 |

## Later (sync / teams — not MVP)
FastAPI + PostgreSQL + pgvector, Cloudflare R2 or S3 for audio, Clerk or Supabase Auth, single VPS (Hetzner/DigitalOcean).

## Rejected alternatives
- **Electron** — TypeScript everywhere but +150–300 MB idle RAM (fails NFR-1).
- **Flutter desktop** — weak system-audio capture on Linux.
- **Microservices** — no benefit on a single machine; more failure points.
- **Bot that joins calls** — visible to participants, needs per-platform APIs, poor on Linux.
