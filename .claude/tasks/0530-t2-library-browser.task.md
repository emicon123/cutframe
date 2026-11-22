# Task 0530: Shared library browser

**Tier:** 2 · **Agents:** `frontend`, `backend` · **Depends on:** `0500`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

One browser for text styles, transitions, filters, effects, animations and stickers.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.44` Categories and animated previews — *?*
- [ ] `F2.45` Search — *?*
- [ ] `F2.46` Favourites — *?*
- [ ] `F2.47` Drag from library to timeline / clip — *?*

## Scope

- **backend:** list library items from bundled effect packages with preview thumbnails/animations over `media://`; favourites persisted in preferences.
- **frontend:** categories, search, favourites, drag from library onto clips, cuts and tracks.

## Done when

- adding a new effect package shows it in the browser without UI code changes
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
