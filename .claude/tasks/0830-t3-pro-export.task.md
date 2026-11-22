# Task 0830: ProRes/DNxHR, AV1, VP9, alpha, sequences, queue

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0650`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Professional delivery formats.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.65` ProRes / DNxHR export (MOV) — *?*
- [ ] `F3.66` AV1 export — *?*
- [ ] `F3.67` VP9 / WebM export — *?*
- [ ] `F3.68` Export with transparent background (ProRes 4444, WebM alpha, PNG sequence) — *?*
- [ ] `F3.69` Image sequence export — *?*
- [ ] `F3.70` Export queue / batch export — *?*
- [ ] `F3.71` Higher frame rates (120 fps) and up to 8K — *?*
- [ ] `F3.72` Audio stems export (per track) — *?*
- [ ] `F3.73` Chapter markers embedded in MP4 — *?*

## Scope

- **backend:** FFmpeg native ProRes/DNxHR encoders, AV1 (hardware or SVT-AV1), VP9 via libvpx (BSD), alpha export, image sequences, stems, MP4 chapters from markers, export queue, 120 fps / 8K limits.
- **devops:** confirm every added encoder library is LGPL-compatible and bundled correctly.

## Done when

- each format exports and re-imports into Cutframe correctly
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
