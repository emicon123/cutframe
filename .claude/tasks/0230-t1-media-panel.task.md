# Task 0230: Media panel with thumbnails and waveforms

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0220`
**Spec:** `docs/design/media-import.md` (§4 routes, §6.3–6.4, §12 task 0230), `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `CLAUDE.md`

## Why

Users pick clips visually; the media bin is an empty placeholder today.

## Features

- [ ] `F1.14` Grid of media with thumbnail, name, duration, type icon — *missing*
- [ ] `F1.15` List view alternative — *missing*
- [ ] `F1.16` Hover-scrub thumbnail preview (skimming) — *missing*
- [ ] `F1.19` Sort and filter (type, name, date, duration) — *missing*
- [ ] `F1.20` Search by name — *missing*
- [ ] `F1.21` Remove media from project (with warning if used) — *missing*
- [ ] `F1.22` Show in file explorer — *missing*
- [ ] `F1.23` Background thumbnail and waveform generation with progress — *missing*

## Scope

- **backend:** bounded `media://` handler pool with deferred responders and an in-flight map (review F12); routes `/poster/<id>`, `/filmstrip/<id>?tile=`, `/waveform/<id>` by id with `?v=` (design §4, §6.3).
- **backend:** content-keyed disk cache under `app_local_data_dir()\cache\media\v1` (atomic writes, version folders, 2 GiB LRU eviction) and the poster/filmstrip/waveform generators; background preview job after import and after open (design §5, §6.4).
- **backend:** HEIC tile-grid stitching (shared engine code; the 0220 probe only decodes the first tile) so HEIC posters show the whole upright photo; 0320 reuses it for image clips.
- **backend:** `RemoveMedia` + `remove_media` (undoable; refuses with `MediaInUse` while clips use it, which can only happen from 0260 on), `reveal_media`.
- **frontend:** grid and list views, hover-scrub skimming over filmstrip tiles, waveform items, sort/filter/search, remove with confirmation, Show in Explorer, background progress.
- **moved out:** F1.17 (preview in the player) → 0340; F1.18 (used-in-timeline badge) and the "warn if used" path of F1.21 → 0260 (design §12).

## Done when

- 100 imported files stay responsive while thumbnails fill in progressively
- thumbnails survive a restart (disk cache) and are invalidated if the source file changes
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
