# Task 0650: Export presets, range and cover image

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0400`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Quality-of-life export features.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.100` Export a range (in/out) only — *?*
- [ ] `F2.101` Save and reuse export presets — *?*
- [ ] `F2.102` Estimated file size — *?*
- [ ] `F2.104` Social presets (vertical 1080×1920, square, etc.) — *?*
- [ ] `F2.105` Video cover / thumbnail: pick a frame or image, add text, embed in file and use in recent projects — *?*

## Scope

- **backend:** range export, size estimate, cover frame embedded in MP4 and used by the home screen.
- **frontend:** presets (save/reuse, social presets), cover picker with text.

## Done when

- an exported MP4 with a cover opens correctly in common players
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
