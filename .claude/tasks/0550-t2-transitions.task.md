# Task 0550: Transitions

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0530`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Transitions between cuts are a core consumer-editor feature.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.21` Add a transition on the cut between two adjacent clips (drag from library or click on cut) — *?*
- [ ] `F2.22` Basic set: cross dissolve, fade to black/white, wipe, slide, push, zoom, blur, spin — *?*
- [ ] `F2.23` Stylised set: glitch, flash, light leak, page curl, ink, shape reveals — *?*
- [ ] `F2.24` Adjustable duration (handles/overlap uses media beyond the cut) — *?*
- [ ] `F2.25` Apply transition to all cuts — *?*
- [ ] `F2.26` Animated preview thumbnails in the transitions library — *?*
- [ ] `F2.27` Transition parameters (direction, softness, colour) — *?*

## Scope

- **backend:** transition model with handles per the design, basic and stylised transition shaders as effect packages, apply-to-all.
- **frontend:** drop on cut, duration handles on the timeline, parameters in the inspector, animated previews in the library.

## Done when

- a transition between clips without enough handle media degrades predictably (documented and tested)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
