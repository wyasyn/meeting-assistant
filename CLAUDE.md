# CLAUDE.md

Claude Code reads this file automatically. The project's full agent instructions live in AGENTS.md so any coding agent can use them.

@AGENTS.md

## Claude Code specifics
- Start every session by reading `docs/08-roadmap.md` to find the current phase and the next unchecked task.
- Use plan mode for any task touching more than 3 files, the database schema, or the audio pipeline. Show the plan before editing.
- After finishing a task: run the checks in AGENTS.md "Definition of done", tick the task in `docs/08-roadmap.md`, and add a line to `docs/09-decisions.md` if you made a design choice.
- Ask before adding a new dependency, changing a public command/event name, or changing the schema.
