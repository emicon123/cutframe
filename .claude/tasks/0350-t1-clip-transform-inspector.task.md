# Task 0350: Clip inspector: transform, crop, opacity, canvas

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0320`, `0250`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Basic look of each clip; the inspector shows only project info today.

## Features

- [ ] `F1.70` Transform: position, scale, rotation — *missing*
- [ ] `F1.71` Non-uniform scale (lock/unlock aspect) — *missing*
- [ ] `F1.72` Opacity — *missing*
- [ ] `F1.73` Flip horizontal / vertical; rotate 90° — *missing*
- [ ] `F1.74` Crop (free and fixed ratios) with crop handles in the preview — *missing*
- [ ] `F1.75` Fit / fill / stretch to canvas — *missing*
- [ ] `F1.76` Canvas background: colour, blur of the clip itself, image — *missing*
- [ ] `F1.84` Reset a property / reset all — *missing*
- [ ] `F1.85` Numeric entry for every slider (type exact values) — *missing*

## Scope

- **backend:** keyframe-ready clip properties per the design (position, scale, rotation, opacity, flip, crop, fit mode) and canvas background (colour, blur, image) in the render graph; set/reset commands as gestures for sliders.
- **frontend:** clip inspector with sliders + numeric entry, reset per property and reset all.

## Done when

- each property renders identically in preview and in a golden-frame test
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
