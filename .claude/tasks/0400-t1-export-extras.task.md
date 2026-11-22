# Task 0400: Audio-only export, frame snapshot, after-export actions

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0390`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Small export variants users expect from day one.

## Features

- [ ] `F1.96` Export audio only (WAV, MP3, AAC) — *missing*
- [ ] `F1.97` Export the current frame as an image (PNG/JPG) — *missing*
- [ ] `F1.98` Open the folder / play the file when the export finishes — *missing*

## Scope

- **backend:** audio-only export (WAV, MP3 via LGPL `libmp3lame` or native, AAC); current-frame PNG/JPG export from the render graph.
- **frontend:** options in the export dialog; open folder / play file buttons when done.

## Done when

- all three work from the export dialog and are covered by tests
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
