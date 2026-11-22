# Task 0760: HSL, curves, colour wheels, scopes, colour match

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0750`, `0540`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Precise grading beyond basic adjustments.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.25` HSL: per-hue-range hue/saturation/lightness — *?*
- [ ] `F3.26` Curves: master, R, G, B, hue-vs-sat, etc. — *?*
- [ ] `F3.27` Colour wheels: lift, gamma, gain, offset — *?*
- [ ] `F3.28` Scopes: waveform, vectorscope, histogram, RGB parade — *?*
- [ ] `F3.29` Colour match between clips — *?*

## Scope

- **backend:** grading shaders as effect packages; scopes computed on the GPU and sent as images over `media://` (never JSON).
- **frontend:** curves and wheels editors, scopes panel.

## Done when

- scopes update during playback without dropping preview frames
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
