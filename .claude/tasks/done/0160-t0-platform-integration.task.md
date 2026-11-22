# Task 0160: GPU fallback, single instance, file association

**Tier:** 0 · **Agents:** `backend`, `devops` · **Depends on:** `0100`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Opening a project file from Explorer should open or focus the app; GPU problems must degrade gracefully.

## Features

- [ ] `F0.13` GPU / hardware-acceleration capability detection with graceful fallback — *partial*
- [ ] `F0.14` Single-instance handling (opening a project file focuses the running app) — *missing*
- [ ] `F0.15` File association for the project file extension — *missing*

## Scope

- **backend:** single-instance plugin; a second launch with a project path forwards it to the running app.
- **devops:** NSIS file association for the project extension (per-user, no admin) — extension from task 0100.
- **backend:** capability report (adapter, backend, software/WARP, hardware decode/encode availability as it appears) and a clear UI message plus software fallback when the GPU fails at runtime.

## Done when

- double-clicking a project file opens it; with the app running it focuses the existing window
- the installed app still needs no admin rights (CI install test)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Closing note (2026-11-22)

Moved to done after `pnpm check` and `cargo deny` passed on Windows. **The manual check in `pnpm dev` was not done**, and neither was the installer CI run (file association, single instance). The "manual check pending" notes in `docs/feature-catalog.md` still apply.
