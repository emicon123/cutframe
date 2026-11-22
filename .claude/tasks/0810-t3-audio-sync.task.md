# Task 0810: Sync clips by audio

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0700` (analysis job queue)
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Syncing a camera clip with audio from a separate microphone or recorder.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.58` Sync clips by audio waveform — *?*

## Scope

- **backend:** audio cross-correlation sync job; sync applied as one `Batch`.
- **frontend:** "Sync by audio" action on the selected clips.

## Done when

- a camera clip and an external recorder clip sync automatically within one frame
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
