# Task 0810: Audio sync and multicam

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0800`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Syncing separately recorded audio/video and switching camera angles.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.57` Multicam editing (sync by audio, switch angles) — *?*
- [ ] `F3.58` Sync clips by audio waveform — *?*

## Scope

- **backend:** audio cross-correlation sync job; multicam clip model (design note first).
- **frontend:** angle viewer and live switching during playback.

## Done when

- two cameras plus an external recorder sync automatically within one frame
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
