# Task 0340: Preview quality, zoom, full screen and skimming

**Tier:** 1 · **Agents:** `frontend`, `backend` · **Depends on:** `0330`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Users need to trade quality for smoothness and see the frame large.

## Features

- [ ] `F1.55` Hover preview: skimming the timeline shows that frame in the preview, with on/off toggle — *missing*
- [ ] `F1.61` Preview quality setting (full / half / quarter resolution) — *missing*
- [ ] `F1.62` Preview fit/zoom levels — *partial*
- [ ] `F1.63` Full-screen preview — *missing*

## Scope

- **backend:** preview resolution scale (full/half/quarter); full-screen presentation of the native preview window.
- **frontend:** quality menu, fit/zoom levels, full-screen toggle (Esc to exit), hover skimming on the timeline with a toggle (hide the native window correctly, ADR-0005 airspace helper, review F11).

## Done when

- switching quality during playback takes effect without a stall
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
