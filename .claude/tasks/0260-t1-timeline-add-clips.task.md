# Task 0260: Put media on the timeline; main and overlay tracks

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0250`, `0230`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Core MVP step: media panel → timeline.

## Features

- [ ] `F1.03` Drag and drop files from the OS directly onto the timeline — *missing*
- [ ] `F1.24` Timeline with a main video track — *partial*
- [ ] `F1.25` Add media to the timeline (drag, "+" button, append at end, insert at playhead) — *missing*
- [ ] `F1.26` Overlay video tracks above the main track (picture-in-picture) — *partial*
- [ ] `F1.27` Audio tracks below the main track — *partial*
- [ ] `F1.28` Tracks created automatically when dropping onto empty space; empty tracks removed — *partial*

## Scope

- **backend:** commands to add clips from media (append, insert at playhead, drop at a time/track), auto-create tracks on drop into empty space and remove empty ones (one undo step via `Batch`).
- **frontend:** drag from media panel and from Explorer onto the timeline with a drop preview; `+` button on media items.

## Done when

- a short edit can be assembled from the media panel with drag and drop; every step undoes in one go
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
