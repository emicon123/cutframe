# Task 0630: Markers, clip labels, ripple trim and selection tools

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0290`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Speed-ups for everyday editing.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.88` Timeline markers (add, colour, label, jump, list) — *?*
- [ ] `F2.89` Clip markers — *?*
- [ ] `F2.92` Rename clips; clip colour labels — *?*
- [ ] `F2.93` Track height and thumbnail size — *?*
- [ ] `F2.94` Set clip duration numerically — *?*
- [ ] `F2.96` Copy / paste attributes (effects, adjustments, transform) to other clips — *?*
- [ ] `F2.97` Range (in/out) selection on the timeline — *?*
- [ ] `F2.98` Select all clips forward / backward from the playhead — *?*
- [ ] `F2.99` Ripple trim — *?*

## Scope

- **backend:** timeline and clip markers, clip rename/colour, numeric duration, paste attributes, in/out range, ripple trim.
- **frontend:** marker UI and list, track height/thumbnail size, select forward/backward.

## Done when

- markers survive save/load and appear as snap targets
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
