# Task 0540: Filters, adjustments and LUTs

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0530`, `0520`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Colour looks are expected on day one.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.28` Filter library (look presets) with intensity slider — *?*
- [ ] `F2.29` Basic adjustments: brightness, contrast, saturation, exposure, temperature, tint — *?*
- [ ] `F2.30` More adjustments: highlights, shadows, whites, blacks, vibrance, fade — *?*
- [ ] `F2.31` Detail: sharpen, clarity, vignette, grain — *?*
- [ ] `F2.32` Auto adjust (one-click auto exposure/colour) — *?*
- [ ] `F2.33` Apply filter/adjustment to all clips — *?*
- [ ] `F2.34` Filter or adjustment as its own segment on a track (adjustment layer affecting everything below for its duration) — *?*
- [ ] `F2.35` Save own adjustment preset — *?*
- [ ] `F2.36` Import LUT (`.cube`) — *?*
- [ ] `F2.37` Before/after comparison in the preview — *?*

## Scope

- **backend:** adjustment shader(s) as effect packages, filter presets with intensity, `.cube` LUT import, adjustment segments on tracks, apply-to-all, user presets, auto adjust (simple histogram-based).
- **frontend:** adjustment panel, filter browser, before/after split in the preview.

## Done when

- every adjustment has a golden-frame test; adjustment segment affects only tracks below it
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
