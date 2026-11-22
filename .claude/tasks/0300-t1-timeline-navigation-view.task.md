# Task 0300: Timeline navigation, zoom and clip visuals

**Tier:** 1 · **Agents:** `frontend`, `backend` · **Depends on:** `0270`, `0230`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Finish timeline ergonomics: zoom controls, keyboard navigation and clip thumbnails/waveforms.

## Features

- [ ] `F1.32` Zoom (Ctrl+wheel, slider, zoom-to-fit) — *partial*
- [ ] `F1.34` Filmstrip thumbnails on video clips — *missing*
- [ ] `F1.35` Waveforms on audio clips and on video clips with audio — *missing*
- [ ] `F1.54` Frame-step (Left/Right), jump to previous/next edit (Up/Down), go to start/end (Home/End) — *missing*

## Scope

- **frontend:** zoom slider and zoom-to-fit (Ctrl+wheel exists); frame step, previous/next edit, Home/End.
- **frontend:** filmstrips and waveforms on clips from `media://`, drawn on the canvas with caching per zoom level.

## Done when

- timeline with 200 clips scrolls and zooms at 60 fps on the Pi
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
