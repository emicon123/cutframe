# Task 0340: Preview quality, zoom, full screen and skimming

**Tier:** 1 · **Agents:** `frontend`, `backend` · **Depends on:** `0330`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Users need to trade quality for smoothness and see the frame large.

## Features

- [ ] `F1.55` Hover preview: skimming the timeline shows that frame in the preview, with on/off toggle — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.61` Preview quality setting (full / half / quarter resolution) — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.62` Preview fit/zoom levels — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.63` Full-screen preview — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.17` Preview a media item in the player before adding it — *partial* (built and tested; manual `pnpm dev` check open) (moved from 0230, `docs/design/media-import.md` §12)

## Scope

Design: `docs/design/render-graph.md` §8 (quality, zoom, full screen) and `docs/design/playback.md` §4.2, §8 (skim, playback target); ADR-0005 amendment of 2026-11-22.

- **backend:** preview resolution scale: composite at `min(canvas, fitted preview size × zoom) × quality` (full/half/quarter) with `set_preview_quality`; zoom levels and pan in the present pass with `set_preview_view`; `set_skim(Option<Time>)` (paused only, no audio); a source-preview mode `set_playback_target(Media { media })` that plays one media asset (single-layer plan, its default audio stream) instead of the timeline (F1.17), keeping the timeline playhead.
- **backend:** full screen needs no engine change: the main window goes full screen and the child window follows the reported rectangle (ADR-0005 amendment 2026-11-22, §5).
- **frontend:** quality menu (persisted as a preference), fit/zoom levels, full-screen toggle (main window full screen through the Tauri window API with the `core:window:allow-set-fullscreen` capability, preview area fills the webview, Esc to exit, Space plays), hover skimming on the timeline with a toggle (hide the native window correctly, ADR-0005 airspace helper, review F11), source preview from the media panel and back to the timeline.

## Done when

- switching quality during playback takes effect without a stall (no decoder restart)
- full-screen preview stays aligned (closes the remaining ADR-0005 spike check)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Status (2026-11-27, branch `claude/project-thread-z58xeb`)

- [x] **Switching quality during playback takes effect without a stall:** render size changes only
  recreate the canvas; a WARP test switches four sizes while playing and counts no extra seek or
  reader; the ignored hardware variant passed on the RTX 3070 Ti.
- [ ] **Full-screen preview stays aligned (ADR-0005 spike check):** built (the child window follows
  the reported rect). Open: checking it by eye in `pnpm dev`, incl. Ctrl+wheel and drag-pan over
  the native preview (input should pass through `HTTRANSPARENT`).
- [ ] **Every feature end-to-end in `pnpm dev` with tests:** engine, src-tauri and Vitest tests for
  all of them. Open: trying quality, zoom, full screen, skimming and source preview by hand.
- [x] **Catalog rows updated:** F1.17, F1.55, F1.61–F1.63 (partial: manual check open).
- [x] **`pnpm check` passes; `pnpm bindings` committed.**
- [x] **Architecture rules:** no model changes; no frames over IPC; integer flicks for skim and
  media time.
