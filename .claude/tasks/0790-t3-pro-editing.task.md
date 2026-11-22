# Task 0790: Insert/overwrite, roll/slip/slide, timecode entry, custom shortcuts

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0630`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Precision editing for experienced users.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.51` Insert vs overwrite edits; three-point editing — *?*
- [ ] `F3.52` Roll, slip and slide edits — *?*
- [ ] `F3.53` Timecode entry for move/seek ("+1:00" style) — *?*
- [ ] `F3.54` Customisable keyboard shortcuts with presets resembling other popular editors — *?*

## Scope

- **backend:** three-point editing, roll/slip/slide as gestures.
- **frontend:** timecode entry, shortcut customisation with presets (no other product's trademarks in preset names).

## Done when

- all trim modes are one undo step and keep linked A/V in sync
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
