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

## ADR-007 (2026-10-02): React 19 + Tailwind v4 instead of React 18
The stack named React 18, but current shadcn/ui targets React 19 and Tailwind v4, and new components assume them. Decision: scaffold with React 19, Tailwind v4 (`@tailwindcss/vite`, no `tailwind.config`) and shadcn's `cn` package in place of clsx + tailwind-merge. TypeScript 6 deprecates `baseUrl`, so the `@/*` alias uses `paths` only.

## ADR-008 (2026-10-02): Placeholder name and bundle identifier
The product name is still open. Decision: `productName` "Meeting Assistant", identifier `dev.meetingassistant.app`, Rust crate `meeting-assistant` (lib `meeting_assistant_lib`). UI reads `APP_NAME` from `app/src/config.ts`. Change all four together once a name is chosen; the identifier must be final before the first public release because it sets data and keychain paths.

## ADR-009 (2026-10-02): Lint and test tooling
Frontend: ESLint 10 flat config with typescript-eslint `strictTypeChecked` + `stylisticTypeChecked`, react-hooks and react-refresh; `no-explicit-any` is an error. Prettier (with `prettier-plugin-tailwindcss` for class order) runs inside `npm run lint`. Vitest + jsdom + Testing Library, coverage via `@vitest/coverage-v8` (NFR-23). Rust: clippy denies `unwrap_used` and `expect_used` through `[lints.clippy]` in Cargo.toml, with `clippy.toml` allowing them in tests, so the AGENTS.md rule is enforced rather than reviewed. Python: ruff rule sets E, W, F, I, B, UP, SIM, and `ruff format --check` is part of the lint command.

## ADR-010 (2026-10-02): Error shape and logging
`AppError` (thiserror) serialises to the contract `{ code, message, retryable }`; a new `internal` code covers failures with no contract code, shows a generic message and logs the detail at debug only (rule 8). Retryable codes: `provider_unavailable`, `invalid_llm_output`, `sidecar_down`. Logs: `tracing-subscriber` with a daily rolling file in `<app_data>/logs/` (7 files kept), stderr in debug builds, default filter `info` overridable by `RUST_LOG`, panics logged by a hook. The UI calls commands only through `call`/`on` in `lib/ipc.ts`, which always reject with an `AppError` instance.

## ADR-011 (2026-10-02): Store encryption, key handling and migrations
`rusqlite` with `bundled-sqlcipher-vendored-openssl`, so every OS builds the same SQLCipher with no system OpenSSL. The key is 32 random bytes (`getrandom`) stored as a binary secret in the OS keychain via `keyring` 4 (service = bundle identifier, account `db-key`; Secret Service over zbus on Linux, no extra system package) and passed as a raw hex key, so there is no passphrase derivation. If the database exists but the key is gone, opening fails with a readable storage error and no new key is made, since that would lock the data out for good. Migrations use a small in-house runner (`include_str!` + `PRAGMA user_version`, one transaction per file) instead of a crate. The connection sits behind a `Mutex` in Tauri state until the job queue needs a pool.

## ADR-012 (2026-10-02): Tray-resident app and start on login
Closing the main window hides it; the app keeps running in the tray so detection can work in the background, and Quit in the tray menu exits. The window starts hidden and is shown in setup unless the process got `--minimized`, which only the login item passes (FR-8.5). Start on login uses `tauri-plugin-autostart`, and the OS login item is the source of truth (no copy in `settings`), so it cannot drift if the user removes it outside the app. `tauri-plugin-single-instance` makes a second launch show the running window instead of opening the encrypted database twice. "Start recording" is in the tray menu but disabled until recording exists (roadmap 1.3). On GNOME the tray needs the AppIndicator extension (Fedora: `gnome-shell-extension-appindicator`); without it a second launch still brings the window back.

## ADR-013 (2026-10-02): PipeWire capture design and spike findings
Design: `AudioBackend` trait in `capture/` with a Linux `PipeWireBackend` (`pipewire` 0.10, Linux-only dependency; other OSes return "unsupported" until their backends land). Each capture owns one thread with its own main loop and two streams: mic (`media.role=Communication`) and system (`stream.capture.sink=true`, the output's monitor). Callbacks run on that thread, not the realtime thread, and push 48 kHz mono f32 frames into a bounded queue, dropping (with a warning) rather than blocking. Frames carry a per-track sample offset from session start.
Spike on Fedora 44, PipeWire 1.6.9, WirePlumber 0.5.18 (`examples/capture_spike.rs`):
- Requesting F32LE/48 kHz/mono works; the adapter resamples and downmixes, also for Bluetooth HFP.
- An idle output still delivers zeros on its monitor, so silence is not a gap.
- Streams without `target.object` follow a change of default output mid-capture (brief pause, then resumes).
- Built-in devices: both tracks complete, equal length.
- Bluetooth headset already in the headset (HFP) profile: both tracks complete.
- Bluetooth headset in A2DP when capture starts: opening the mic makes WirePlumber switch to HFP and both streams stall for seconds while still linked and "running". `pw-record` shows the same, so it is system behaviour, not ours.
Consequences: the track clock jumps offsets forward to wall-clock time on gaps over 100 ms, so tracks stay aligned and later steps can fill silence; a watchdog reconnects a stream with no data for 2 s (it recovered the mic in 3 of 4 runs; the system track can stay silent until the headset settles). In real calls the meeting app usually holds the mic first, so HFP is already active. The recording UI (1.3) must show live levels so a stall is visible. Not tested: a Bluetooth device disconnecting mid-capture.

## ADR-014 (2026-10-02): Encrypted 10 s Opus chunks and crash recovery
Each track is cut into 10 s chunks on a fixed timeline (chunk N starts at (N-1) x 10 s), with capture gaps encoded as silence so mic and system stay aligned and the pipeline can derive times from the index. Every chunk is its own Ogg Opus stream: the encoder is reset per chunk and flushed with one silent frame, and the last granule position trims the padding, so a chunk decodes and plays alone and can be sent to a provider as a standard file. We feed 48 kHz and cap the encoder at wideband (16 kHz audio, VOIP mode, 24 kbps, DTX), so libopus resamples and silence costs a few hundred bytes. libopus comes from the `opus` crate, built from bundled source with cmake, so no system package is needed on any OS. Chunks are sealed with AES-256-GCM using the database key (docs/04), a random nonce per file and `<meeting_id>/<track>/<N>` as associated data so files cannot be swapped. Writes go to `.tmp`, are synced and renamed, so a crash loses at most the chunk in progress: a `kill -9` at 17 s on Fedora kept the first 10 s of both tracks intact. At startup, meetings still in `recording` become the new `interrupted` status (no migration: the column has no CHECK), their audio is kept and their length is read from the last chunk. The audio dir is stored relative to `<app_data>` so moving the data folder keeps it valid. Fedora check: built-in devices gave complete, aligned tracks with a test tone continuous across chunk boundaries; the Bluetooth headset still stalls as in ADR-013, and those gaps are saved as silence.

## ADR-015 (2026-10-02): Manual recording controls, pause and the indicator
A `RecordingService` owns the one active recording and is the only way to start one: the window button and the tray menu call it; nothing else does (rule 1). Pause closes the capture session so the devices are released and the OS mic indicator turns off; resume opens a new session and continues both tracks from the end of the longer one, so paused time is not stored and the tracks stay aligned (the shorter one is padded with silence). `elapsedMs` therefore counts recorded time only. Stop flushes the last chunks and sets the meeting to `processing` for the job queue (1.7). A watchdog checks the recorder every 500 ms; if capture or saving died by itself the meeting becomes `interrupted` (audio kept) and `recording:state` carries a readable `error`. Quitting from the tray saves a running recording first. Levels are RMS per track over 100 ms, `null` when a device sent nothing, so a stalled Bluetooth track (ADR-013) is visible rather than looking silent. The tray shows the state with a red dot painted onto the app icon at runtime (grey while paused, no extra asset), the label "Recording" or "Paused" next to it on Linux, and Start/Stop and Pause/Resume menu items; the window shows the same red dot and label in a bar fixed to the top. Device choice and the FR-1.4 global hotkey come later (FR-8.3 settings). Commands run on a blocking worker so device setup and file flushing never stall the UI thread. Tray-started recordings are titled "Recording"; the window passes a local-time title.

## Open questions (owner to decide)
- [ ] Product name and bundle identifier.
- [ ] Default cloud STT: Gemini alone, or Deepgram/AssemblyAI for better diarization?
- [ ] Personal tool only, or team accounts later (affects sync/pricing)?
- [ ] Business model: free with own key, or paid tier with bundled credits?
- [ ] Flatpak sandbox vs process-watching for detection — accept host permissions or rely on extension + PipeWire only?
