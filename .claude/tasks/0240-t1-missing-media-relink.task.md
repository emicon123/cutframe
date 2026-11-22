# Task 0240: Missing media detection and relink

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0230`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Projects break when media moves; relink is expected in every editor.

## Features

- [ ] `F0.25` Detect missing media on open; relink dialog (single file and whole folder) — *missing*

## Scope

- **backend:** detect missing assets on open; relink one file or search a folder by name + fingerprint; relink is undoable.
- **frontend:** relink dialog; missing media shown with an offline placeholder in bin, timeline and preview.

## Done when

- moving the media folder and relinking it restores the project with one action
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
