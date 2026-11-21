# Task: Align agent prompts with the code and ADRs

**Owner:** orchestrator (main session). `.claude/agents/` is not owned by any specialist agent.
**Source:** `docs/architecture-review.md` F47 (P0 #5).
**Do this first:** the specialist agents read these prompts before every other task.

## Problem

`backend.md` and `frontend.md` name paths and APIs that do not exist, so agents guess:

| Prompt | Says | Actually |
|---|---|---|
| `backend.md:98` | `app-core/src/migrations/` | `crates/app-core/src/serialization.rs` (`migrate`) |
| `backend.md:103` | `ipc/commands/*.rs` | `src-tauri/src/ipc/commands.rs` (one file) |
| `backend.md:110` | `project_changed { revision, view }` | event `project-changed` carrying `ProjectState` (ADR-0003 §6-7) |
| `frontend.md:74` | `lib/timecode.ts` | `ui/src/lib/time.ts` |
| `frontend.md:85, 93-94` | `hidePreview` / `showPreview` | `setPreviewVisible` (bindings) |

## Scope

- Fix the references above. Re-check the line numbers first; they are from the review.
- Point both prompts (and `architect.md`, `devops.md` where relevant) at `docs/architecture.md` and the
  ADRs 0002–0006, and add the rules they rely on in one line each: kebab-case event names, Rust sends
  keys not user-facing text (ADR-0003 §5), one user gesture = one undo entry (ADR-0006), sync model
  commands on the main thread (ADR-0004).
- Do not change `CLAUDE.md` here; its suggested changes are a separate user decision.

## Done when

- every path or API named in `.claude/agents/*.md` exists in the repo or is clearly marked as target
- one `docs:`/`chore:` commit
