# Task 0620: Decide and build bundled content libraries

**Tier:** 2 · **Agents:** **user**, `backend`, `frontend`, `devops` · **Depends on:** `0530`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Stickers, sound effects and music need licensed content — a product/legal decision before any code.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.72` Sticker library (static and animated) — *?*
- [ ] `F2.86` Built-in sound effects library — *?*
- [ ] `F2.87` Built-in royalty-free music library — *?*

## Scope

- **user:** decide source and licence (own assets, CC0 packs, a licensed provider, or none). No reference-product assets (rule 8).
- **devops/backend/frontend:** if in scope, ship as downloadable or bundled packages browsed in the shared library; otherwise mark the rows `n/a`.

## Done when

- each row is `done` or `n/a` with the reason
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
