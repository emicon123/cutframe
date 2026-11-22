# Task 0780: Smooth slow motion, motion blur, denoise, lens fixes

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0770`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Image quality and retiming tools.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.35` Smooth slow motion (optical-flow frame interpolation) — *?*
- [ ] `F3.36` Frame blending for speed changes — *?*
- [ ] `F3.37` Motion blur — *?*
- [ ] `F3.42` Video denoise — *?*
- [ ] `F3.43` Deflicker — *?*
- [ ] `F3.44` Lens distortion correction — *?*

## Scope

- **backend:** optical-flow interpolation, frame blending, motion blur, temporal denoise, deflicker, lens distortion correction — GPU shaders or analysis jobs as appropriate.

## Done when

- 0.25× optical-flow slow motion renders without obvious artefacts on a test clip
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
