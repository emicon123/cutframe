# Task 0740: Beat detection and cut to beat

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0730`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Music-synced editing.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.20` Automatic beat detection on music — *?*
- [ ] `F3.21` Auto-cut to the beat (place cuts on beat markers) — *?*

## Scope

- **backend:** onset/beat detection job writing beat markers; auto-cut to beats as one `Batch`.

## Done when

- auto-cut to beat on a 120 BPM track lands cuts within one frame of the beat
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
