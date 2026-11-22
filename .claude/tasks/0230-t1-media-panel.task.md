# Task 0230: Media panel with thumbnails and waveforms

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0220`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Users pick clips visually; the media bin is an empty placeholder today.

## Features

- [ ] `F1.14` Grid of media with thumbnail, name, duration, type icon — *missing*
- [ ] `F1.15` List view alternative — *missing*
- [ ] `F1.16` Hover-scrub thumbnail preview (skimming) — *missing*
- [ ] `F1.17` Preview a media item in the player before adding it — *missing*
- [ ] `F1.18` "Used in timeline" badge — *missing*
- [ ] `F1.19` Sort and filter (type, name, date, duration) — *missing*
- [ ] `F1.20` Search by name — *missing*
- [ ] `F1.21` Remove media from project (with warning if used) — *missing*
- [ ] `F1.22` Show in file explorer — *missing*
- [ ] `F1.23` Background thumbnail and waveform generation with progress — *missing*

## Scope

- **backend:** background thumbnail/filmstrip/waveform generation into the cache, served over `media://` (never JSON IPC); remove-media command (undoable, warns if used).
- **frontend:** grid and list views, hover-scrub skimming, preview in the player before adding, used-in-timeline badge, sort/filter/search, show in Explorer.

## Done when

- 100 imported files stay responsive while thumbnails fill in progressively
- thumbnails survive a restart (disk cache) and are invalidated if the source file changes
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
