# Task 0800: Bins, multiple timelines, versions, archive, interchange

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0640`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Organising larger projects and moving them between tools.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.55` Media folders/bins, tags, ratings — *?*
- [ ] `F3.56` Multiple timelines/sequences in one project — *?*
- [ ] `F3.59` Project versions / snapshots — *?*
- [ ] `F3.60` Collect files: copy project + all media into a folder (archive) — *?*
- [ ] `F3.61` Interchange: import/export OpenTimelineIO / FCPXML / EDL — *?*

## Scope

- **architect:** multiple timelines per project and project-file migration (ADR-0002); interchange format choice (OpenTimelineIO first).
- **backend/frontend:** folders/tags/ratings in the media panel, timeline tabs, project snapshots, collect-files archive, OTIO/FCPXML/EDL import/export.

## Done when

- an archived project opens on another machine with all media
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
