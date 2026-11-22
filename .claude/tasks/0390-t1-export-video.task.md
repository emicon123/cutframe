# Task 0390: Export to MP4 (H.264/HEVC + AAC)

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0380`, `0370`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Last MVP step: get the edit out of the app.

## Features

- [ ] `F1.86` Export dialog: file name, destination folder — *missing*
- [ ] `F1.87` Resolution: 480p, 720p, 1080p, 2K, 4K (keeping project aspect) — *missing*
- [ ] `F1.88` Frame rate: 24, 25, 30, 50, 60 fps — *missing*
- [ ] `F1.89` Bitrate: low / recommended / high / custom — *missing*
- [ ] `F1.90` H.264 export in MP4 — *missing*
- [ ] `F1.91` HEVC export in MP4/MOV — *missing*
- [ ] `F1.92` AAC audio in the export — *missing*
- [ ] `F1.93` Export uses the same render graph as preview — *missing*
- [ ] `F1.94` Progress with ETA, cancel, background export while editing — *missing*
- [ ] `F1.95` Hardware-accelerated encoding with software fallback — *missing*
- [ ] `F1.99` Clear error when export fails (disk full, encoder missing) — *missing*

## Scope

- **backend:** export job per the design; hardware encoders with software fallback; progress/ETA/cancel events; disk-full and missing-encoder errors.
- **frontend:** export dialog (name, folder, resolution, fps, bitrate, codec), progress view, editing stays possible during export.

## Done when

- a 1-minute 1080p30 edit exports on a machine without a discrete GPU (OpenH264 or Media Foundation) and plays in standard players
- golden-frame comparison of exported frames vs preview frames
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
