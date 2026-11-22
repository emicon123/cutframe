# Task 0270: Select, move, trim and snap clips

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Basic direct manipulation on the canvas timeline.

## Features

- [ ] `F1.36` Clip label (name, duration, speed badge, effect badges) — *partial*
- [ ] `F1.37` Select a clip; multi-select with Ctrl/Shift; marquee (rubber-band) selection — *missing*
- [ ] `F1.38` Select all (Ctrl+A); deselect (Esc) — *missing*
- [ ] `F1.39` Move clips by drag within and between tracks — *missing*
- [ ] `F1.40` Trim start/end by dragging clip edges, limited by source length — *missing*
- [ ] `F1.45` Snapping to playhead, clip edges, markers, with on/off toggle — *missing*

## Scope

Design: `docs/design/timeline-editing.md` (§4.2–4.3, §6.1–6.2, §12).

- **backend:** `move_clips` (anchor + absolute start + `TrackTarget`; overlap is rejected; reorder on the magnetic main track) and `trim_clip` (absolute edge time, clamped to source length, one frame, neighbours and linked clips) as ADR-0006 gestures (`Update` per frame, `Commit` on pointer-up, cancel on Escape); linked clips follow; emptied tracks removed on commit only.
- **frontend:** hit-testing in `features/timeline/` (pure geometry functions with Vitest); selection in a UI-only `selectionStore` (click, Ctrl/Shift, marquee, Ctrl+A, Esc; linked groups selected together; pruned on `project-changed`); drag to move between tracks, edge trim; `snapping.ts` (playhead/edges/0, 8 px, frame grid, Alt bypass) with toggle as a UI preference; clip labels with duration.

## Done when

- moving or trimming a clip is one undo step regardless of drag length
- snapping can be toggled and holds within a few pixels at any zoom
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
