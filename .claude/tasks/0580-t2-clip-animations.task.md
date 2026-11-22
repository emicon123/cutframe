# Task 0580: Clip in/out/combo animations

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0520`, `0530`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

One-click motion presets for clips and photos.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.48` Clip in animations (fade, zoom, slide, spin, bounce, blur in) — *?*
- [ ] `F2.49` Clip out animations — *?*
- [ ] `F2.50` Combo / loop animations — *?*
- [ ] `F2.51` Animation duration — *?*
- [ ] `F2.52` Ken Burns pan-and-zoom for images — *?*

## Scope

- **backend:** animation presets as data (generated keyframes or animation effects per the design), Ken Burns for images.
- **frontend:** animation tab in the inspector with duration.

## Done when

- in + out animations on one clip do not overlap incorrectly when the clip is short
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
