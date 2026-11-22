# Task 0110: Autosave and crash recovery

**Tier:** 0 · **Agents:** `backend`, `frontend` · **Depends on:** `0100`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Users lose work when the app crashes; consumer editors save continuously.

## Features

- [ ] `F0.21` Autosave at a configurable interval — *missing*
- [ ] `F0.22` Crash recovery: offer to restore the last autosave — *missing*

## Scope

- **backend:** autosave to the app-data recovery folder on a background thread, from the `ArcSwap` snapshot (never block the main thread or hold the document lock while writing). Interval from preferences (default 1 min) and only when dirty.
- **backend:** on startup, detect a recovery file newer than the saved project; command to restore or discard it.
- **frontend:** recovery prompt on startup listing what can be restored.

## Done when

- killing the process mid-edit and restarting offers recovery with at most one interval of loss
- autosave never writes over the user's own project file
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Closing note (2026-11-22)

Moved to done after `pnpm check` and `cargo deny` passed on Windows. **The manual check in `pnpm dev` was not done**, and neither was the installer CI run (file association, single instance). The "manual check pending" notes in `docs/feature-catalog.md` still apply.
