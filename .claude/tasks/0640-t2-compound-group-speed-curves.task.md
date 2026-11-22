# Task 0640: Compound clips, groups and speed curves

**Tier:** 2 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0630`, `0370`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Structural timeline features with model and render-graph impact.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.90` Compound clip: combine selected clips into one; open/edit; uncombine — *?*
- [ ] `F2.91` Group clips (move together without nesting) — *?*
- [ ] `F2.95` Speed curves: presets (montage, hero, bullet, jump cut, flash in/out) and custom curve — *?*

## Scope

- **architect (short design):** nested timelines in the model and render graph; groups; speed-curve mapping (source time as a function of timeline time, audio implications).
- **backend/frontend:** implement combine/uncombine/open, grouping, speed-curve presets and custom curve editor.

## Done when

- speed curves stay frame-accurate in export
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
