# Task 0360: Direct manipulation in the preview

**Tier:** 1 · **Agents:** `frontend`, `backend` · **Depends on:** `0350`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Dragging and scaling in the player is the main way people place overlays.

## Features

- [ ] `F1.64` Direct manipulation in the preview: drag to move, corner handles to scale, handle to rotate — *missing*
- [ ] `F1.65` Selecting an element in the preview selects its clip on the timeline — *missing*
- [ ] `F1.66` Snapping guides in the preview (centre, edges) — *missing*

## Scope

- **architect (short note):** pointer input over the native preview window (review: input pass-through decision, ADR-0005).
- **frontend/backend:** handles overlay for the selected clip, drag/scale/rotate as gestures, click to select the clip under the pointer, snapping guides.

## Done when

- move/scale/rotate in the preview equals the inspector values; one undo step per drag
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
