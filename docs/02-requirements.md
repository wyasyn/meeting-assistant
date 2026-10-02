# 02 — Requirements

Priority (MoSCoW): **M** must (MVP) · **S** should · **C** could · **W** won't now.

## Functional requirements

### FR-1 Meeting detection and recording control
| ID | Requirement | P |
| --- | --- | --- |
| FR-1.1 | Detect meeting start from desktop apps (Zoom, Slack, Teams, Discord) via running processes and their audio streams. | M |
| FR-1.2 | Detect browser meetings (Meet, Zoom web, Jitsi) via the browser opening the microphone and, later, a browser extension reporting the tab. | M |
| FR-1.3 | On detection, show a system notification: Record / Not now / Never for this app. | M |
| FR-1.4 | Start, pause, resume, stop from the tray icon and a global hotkey. | M |
| FR-1.5 | Visible recording indicator for the whole recording. | M |
| FR-1.6 | Detect meeting end (app closed, mic released, 2+ min silence) and prompt to stop. | M |
| FR-1.7 | Per-app rules: always ask / always record / never record. | S |
| FR-1.8 | Google Calendar: name the meeting, list attendees, pre-arm detection. | S |
| FR-1.9 | Manual in-person mode (microphone only). | S |
| FR-1.10 | One-click consent message to paste into meeting chat. | S |

### FR-2 Audio capture
| ID | Requirement | P |
| --- | --- | --- |
| FR-2.1 | Capture microphone and system output as two separate tracks. | M |
| FR-2.2 | Mic track = the user ("me"). | M |
| FR-2.3 | Choose input/output devices incl. Bluetooth. | M |
| FR-2.4 | Store audio locally as Opus with a retention period. | M |
| FR-2.5 | Keep recording offline; process when the network returns. | M |
| FR-2.6 | Echo suppression so remote voices leaking into the mic aren't attributed to the user. | S |
| FR-2.7 | Import an existing audio/video file. | S |

### FR-3 Transcription and speaker identification
| ID | Requirement | P |
| --- | --- | --- |
| FR-3.1 | Timestamped transcript with word-level timing. | M |
| FR-3.2 | Diarize the system track: Speaker 1, 2, 3… | M |
| FR-3.3 | Rename a speaker once; applies to all their lines. | M |
| FR-3.4 | Suggest names from calendar attendees and opt-in voice profiles. | S |
| FR-3.5 | Live captions during the meeting. | S |
| FR-3.6 | English first; handle mixed English/Luganda as well as the engine allows. | S |
| FR-3.7 | Edit transcript text; custom vocabulary. | S |
| FR-3.8 | Click a transcript line to play that audio. | M |

### FR-4 Summaries and reports
| ID | Requirement | P |
| --- | --- | --- |
| FR-4.1 | Summary: purpose, key points, decisions, open questions. | M |
| FR-4.2 | Action items with owner, due date if stated, link to transcript moment. | M |
| FR-4.3 | Full report: summary + action items + scores + coaching. | M |
| FR-4.4 | Templates: client discovery, standup, sprint review, interview, one-on-one, sales call. | S |
| FR-4.5 | Draft follow-up email. | S |
| FR-4.6 | Regenerate a section with a custom instruction. | S |
| FR-4.7 | Chapter timeline (topics with start times). | C |

### FR-5 Scoring and coaching (details: `07-scoring.md`)
| ID | Requirement | P |
| --- | --- | --- |
| FR-5.1 | Engagement score. | M |
| FR-5.2 | Value-of-discussion score. | M |
| FR-5.3 | My-performance score. | M |
| FR-5.4 | Productivity score. | M |
| FR-5.5 | 3–5 improvement suggestions, each tied to a score and a transcript moment. | M |
| FR-5.6 | Score trends over time, filter by meeting type/client. | S |
| FR-5.7 | Personal goals (e.g. talk ratio < 50%). | C |

### FR-6 Library, search and Ask
| ID | Requirement | P |
| --- | --- | --- |
| FR-6.1 | Meeting list: date, length, participants, source app, scores. | M |
| FR-6.2 | Full-text search across transcripts and summaries. | M |
| FR-6.3 | Tags; group by client/project. | S |
| FR-6.4 | Ask questions across meetings with linked answers. | S |
| FR-6.5 | Action items tracked across meetings. | S |
| FR-6.6 | Pre-meeting brief for recurring attendees. | C |

### FR-7 Export and integrations
| ID | Requirement | P |
| --- | --- | --- |
| FR-7.1 | Export Markdown, PDF, DOCX, SRT, JSON. | M |
| FR-7.2 | Copy summary formatted for Slack/email. | M |
| FR-7.3 | Send to Google Drive, Notion, webhook. | C |
| FR-7.4 | Push action items to Trello, Jira, Todoist, GitHub Issues. | C |

### FR-8 Settings and AI providers
| ID | Requirement | P |
| --- | --- | --- |
| FR-8.1 | Bring-your-own API key (Gemini first), stored in OS keychain. | M |
| FR-8.2 | Offline mode: local Whisper + Ollama. | S |
| FR-8.3 | Language, summary length, default template, retention. | M |
| FR-8.4 | Estimated API cost per meeting. | S |
| FR-8.5 | Start on login, minimised to tray. | M |

### FR-9 Extras
| ID | Requirement | P |
| --- | --- | --- |
| FR-9.1 | Highlight hotkey; marks pinned in summary. | S |
| FR-9.2 | Private live notes merged into report. | C |
| FR-9.3 | Redaction of phones, emails, money in exports. | C |
| FR-9.4 | Sentiment/energy timeline. | C |
| FR-9.5 | Meeting cost estimate. | C |
| FR-9.6 | Weekly digest. | C |

## Non-functional requirements
| ID | Area | Requirement | Measure |
| --- | --- | --- | --- |
| NFR-1 | Performance | Small idle footprint | <150 MB RAM, <1% CPU idle |
| NFR-2 | Performance | Recording doesn't slow the call | <5% CPU recording (4-core laptop) |
| NFR-3 | Performance | Fast detection | Prompt within 10 s of joining |
| NFR-4 | Performance | Fast results | Transcript + summary ≤3 min for 60 min (cloud) |
| NFR-5 | Performance | Live captions | <2 s delay |
| NFR-6 | Reliability | No audio lost | 10 s chunks; recover after crash/power cut |
| NFR-7 | Reliability | Retryable processing | Failed jobs retry; audio kept until success |
| NFR-8 | Reliability | Long meetings | Up to 4 hours |
| NFR-9 | Accuracy | Usable transcripts | WER <15% on clear English |
| NFR-10 | Accuracy | Correct speakers | "Me" 99% (mic track); others 85%+ with 2–6 people |
| NFR-11 | Privacy | Local by default | Only the chosen AI provider receives data |
| NFR-12 | Privacy | Never silent | No recording without user action/rule; indicator always visible |
| NFR-13 | Security | Encrypted at rest | SQLCipher + encrypted audio; keys in OS keychain |
| NFR-14 | Security | Encrypted in transit | TLS 1.2+ |
| NFR-15 | Security | Deletion | Delete one, a range, or everything incl. voice profiles |
| NFR-16 | Legal | Consent | First-run consent notice, per-meeting message, opt-in voice profiles; Uganda DPPA 2019, GDPR |
| NFR-17 | Portability | One codebase | Fedora (Wayland/X11, PipeWire), Ubuntu, Windows 10+, macOS 13+ |
| NFR-18 | Portability | Packaging | Flatpak, RPM, AppImage; later MSI, DMG |
| NFR-19 | Usability | Quick setup | First recording within 3 min of install |
| NFR-20 | Usability | No nagging | One prompt per meeting |
| NFR-21 | Accessibility | Keyboard + screen reader | Full keyboard nav, labels, light/dark, WCAG 2.1 AA |
| NFR-22 | Maintainability | Swappable providers | STT/LLM behind one interface |
| NFR-23 | Maintainability | Tested | ≥70% coverage on core; CI on 3 OSes |
| NFR-24 | Scalability | Fast library | Search <1 s across 1,000 meetings |
| NFR-25 | Updates | Safe updates | Signed auto-updates with rollback |
| NFR-26 | Cost | Visible cost | Shown per meeting; target ≤US$0.30/hour |
| NFR-27 | Sync-ready | Future sync | UUIDs + timestamps on all rows |
