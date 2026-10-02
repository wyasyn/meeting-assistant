# 01 — Vision, users and stakeholders

## Vision
After every call, you should know what was decided, who owes what, and how to run the next meeting better — without taking a single note. The app sits quietly in the tray, notices when a meeting starts, asks once, and does the rest on your own machine.

## Problem
- Notes during calls are incomplete and pull attention away from the conversation.
- Action items get lost; follow-ups are late or never sent.
- People rarely get objective feedback on how they run or contribute to meetings.
- Existing note-takers join calls as visible bots, are cloud-only, or ignore Linux.

## Product principles
1. **Ask, never assume** — consent before every recording unless the user set a rule.
2. **Local-first, private by default** — data lives on the device; the user picks the AI provider and supplies their own key.
3. **Invisible until needed** — no bot joins the call; the app works from the computer's own audio.
4. **Explain every number** — scores show their evidence and the transcript moments behind them.
5. **Linux is first-class** — Fedora/Wayland is the primary platform, not an afterthought.

## Target users
| Persona | Context | Main need |
| --- | --- | --- |
| Developer / agency owner (primary, the builder himself) | Many client calls, standups, discovery calls | Accurate action items, client history, self-coaching |
| Freelancer / consultant | Client calls across Zoom, Meet, WhatsApp | Proof of what was agreed, quick follow-up emails |
| Team lead / manager | Standups, one-on-ones, sprint reviews | Meeting productivity, talk-time balance, decisions log |
| Small agency / startup team (later) | Shared client meetings | Shared library, sync (out of MVP) |

## Stakeholders
| Stakeholder | Interest | Implication for the build |
| --- | --- | --- |
| Owner / developer (Yasin) | Ship a usable MVP on Fedora, grow into a product | Small, testable phases; low running cost |
| End users | Reliable capture, useful summaries, privacy | NFRs on reliability, privacy, accuracy |
| Other meeting participants (non-users) | Being informed they are recorded; their data handled fairly | Consent prompt, consent message, deletion, opt-in voice profiles |
| AI providers (Gemini, Deepgram, AssemblyAI, local models) | API terms, rate limits | Provider trait, retries, cost display |
| OS / distribution platforms (Flathub, Fedora, Microsoft, Apple) | Packaging, sandbox and permission rules | Flatpak portals, signed builds, mic/screen permissions |
| Regulators | Uganda Data Protection and Privacy Act 2019, GDPR, call-recording consent laws | Consent UX, encryption, retention, delete-all |

## Goals (first 6 months)
- G1: Record and transcribe any desktop meeting on Fedora with correct "me" labelling.
- G2: Summary, action items and four scores within 3 minutes of a 60-minute call.
- G3: The owner uses it daily for his own calls (dogfooding) for 4+ weeks.
- G4: Windows build working; macOS build started.

## Non-goals (for now)
- Joining meetings as a bot participant; recording video or screen.
- Mobile app (data model stays sync-ready).
- Team accounts, cloud sync, billing.
- Real-time AI coaching during the call.

## Success metrics
- ≥95% of meetings detected and prompted within 10 s.
- 0 lost recordings in 4 weeks of daily use.
- User edits ≤10% of action items before using them.
- Median processing time ≤3 min for a 60-minute meeting (cloud mode).
- Average API cost ≤ US$0.30 per meeting hour.
