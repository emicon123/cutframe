# Task 0800: Project snapshots and collect-files archive

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0640`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Keeping project history and archiving a finished video together with its media.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.59` Project versions / snapshots — *?*
- [ ] `F3.60` Collect files: copy project + all media into a folder (archive) — *?*

## Scope

- **architect:** where snapshots live and how they relate to autosave (task 0110).
- **backend/frontend:** project snapshots, collect-files archive.

## Done when

- an archived project opens on another machine with all media
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
