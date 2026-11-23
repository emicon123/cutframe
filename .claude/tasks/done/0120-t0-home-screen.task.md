# Task 0120: Home screen and recent projects

**Tier:** 0 · **Agents:** `backend`, `frontend` · **Depends on:** `0100`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The app opens straight into an untitled editor; there is no way to see or manage projects.

## Features

- [ ] `F0.16` Home screen: create project, recent projects (cover thumbnail, name, duration, last modified) — *missing*
- [ ] `F0.23` Rename / duplicate / delete project from the home screen — *partial*

## Scope

- **backend:** recent-projects list in app data (path, name, duration, modified, cover thumbnail via `media://`), pruned when files are missing; commands to list, rename, duplicate and delete (delete = move to recycle bin, not permanent).
- **frontend:** home screen with New project, Open, and the recent list (grid with cover, name, duration, date); context actions rename/duplicate/delete; back to home from the editor.

## Done when

- opening a recent project is one click; a moved/deleted file shows as missing with a remove option
- delete asks for confirmation and is recoverable from the recycle bin
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Closing note (2026-11-23)

Moved to done after `pnpm check` and `cargo deny` passed on Windows. **The manual check in `pnpm dev` was not done** (real Recycle Bin move, Show in Explorer, cover images). The "manual check pending" notes in `docs/feature-catalog.md` still apply.
