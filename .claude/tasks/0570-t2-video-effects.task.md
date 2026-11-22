# Task 0570: Video effects library

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0530`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Stylised effects (glitch, shake, blur…) are a large part of the editing style we target.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.38` Effects library: blur, shake, zoom pulse, glitch, VHS, RGB split, mirror, kaleidoscope, film grain, light leaks, sparkle, retro, etc. — *?*
- [ ] `F2.39` Effect parameters in the inspector — *?*
- [ ] `F2.40` Effect applied to a clip vs. effect segment on its own track — *?*
- [ ] `F2.41` Stack multiple effects; reorder; enable/disable — *?*
- [ ] `F2.42` Mosaic / pixelate / blur region (privacy blur) — *?*
- [ ] `F2.43` Frames and borders — *?*

## Scope

- **backend:** a first set of effect packages, effect stacks (reorder, enable/disable), effect segments on tracks, region mosaic/blur, frames/borders.
- **frontend:** effect parameters in the inspector, stack UI.

## Done when

- at least 15 effects shipped as data; adding one requires no Rust change
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
