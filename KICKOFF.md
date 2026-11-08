# Foreman rewrite — kickoff for Claude Code

## Setup (once)
1. Create a new empty GitHub repo (e.g. `Hellsparks/Foreman`) and clone it.
2. Copy this folder's `docs/` into the repo root (`docs/01…04`).
3. Open Claude Code in the repo and paste the prompt below.

## Prompt — session 1
```
You are starting a full rewrite of Foreman, a 3D-printer fleet manager.

Read every file in docs/ before doing anything. docs/04-claude-code-phase0-1-scaffold-spoolman.md is your task list; docs/01–03 are the background and decisions. Decisions in the docs are final — don't re-open them; if something is impossible or contradicts upstream source, write it in the "Corrections log" at the bottom of doc 04 and tell me.

Reference repos (read-only, clone into vendor/ and gitignore it):
- https://github.com/Hellsparks/Foreman-overview — the old Node app. Read for behaviour only, never copy its structure.
- https://github.com/Hellsparks/Fieldwork-designtemplate — the design system. Copy DESIGN.md, tokens.css and AGENTS.md unmodified into frontend/src/design/.
- https://github.com/Donkie/Spoolman at tag v0.27.1 — the API we must be compatible with (Phase 1).

My machine: CachyOS (Arch), KDE, Wayland, NVIDIA, fish shell. Every command you give me or put in docs must work in fish. No bash-only scripts.

Do session 1 from the session plan in doc 04 (boxes 0.1 and 0.2). Show me a short plan first, then do it. Run fmt/clippy/tests before committing. Tick the boxes in doc 04 and commit when done, then tell me what you did and what session 2 will do.
```

## Prompt — every later session
```
Read docs/04 and do the next session in the session plan. Same rules as before: short plan first, tests before commit, tick boxes, log corrections.
```

## What you need to hand over later
- **Session 4–5 (Phase 1):** a copy of your current Spoolman database file (`spoolman.db`), for the migration test. Stop Spoolman before copying it.
- **Phase 1 Moonraker smoke test:** the address of one Klipper printer you're happy to point at the test server for a few minutes.

## Not for Claude Code
- `docs/03-ui-design-brief.md` is also what you paste into a Claude Design session (with Fieldwork DESIGN.md + tokens.css attached), one batch at a time. Bring the hand-back notes from each batch to the planning project.
