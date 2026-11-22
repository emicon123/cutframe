# Task 0600: Own stickers and shapes

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0580`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Overlays from the user's own files and simple vector shapes.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.73` Import own stickers (PNG, GIF, APNG, animated WebP) — *?*
- [ ] `F2.74` Sticker in/out/loop animations — *?*
- [ ] `F2.75` Shapes (rectangle, circle, arrow, line) with fill/stroke — *?*

## Scope

- **backend:** animated GIF/APNG/WebP decode as clips, sticker animations (reuse clip animations), vector shapes with fill/stroke in the render graph.
- **frontend:** sticker import, shape tools.

## Done when

- animated stickers loop correctly at any project frame rate
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
