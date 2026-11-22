# Task 0100: New, save, open and project settings

**Tier:** 0 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** none
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Nothing a user makes survives a restart today. Serialization (`app-core/src/serialization.rs`) exists, but there is no command or UI to use it.

## Features

- [ ] `F0.17` Create a new project with default settings — *partial*
- [ ] `F0.18` Project settings: canvas resolution, aspect ratio, frame rate — *partial*
- [ ] `F0.19` Save project (explicit save + Save As) — *partial*
- [ ] `F0.20` Open project from file and from the recent list — *partial*
- [ ] `F0.26` Unsaved-changes prompt on close — *missing*

## Scope

- **architect:** move ADR-0002 (project file format) to Accepted or amend it; pick a working file extension (H5: final name is still open, so keep it in one constant).
- **backend:** `Project::validate` on load (clip invariants, review F07) and atomic save (write temp + rename, ADR-0002).
- **backend:** commands `new_project`, `open_project(path)`, `save_project`, `save_project_as(path)`, `set_project_settings` (undoable; resolution, aspect, frame rate). Dirty flag = revision differs from last saved revision; include it in `ProjectState`.
- **backend:** on project replace, the UI store must accept the new state (revision monotonic per session, done in task 0003; add a test).
- **frontend:** File actions (New, Open, Save, Save As) with native dialogs, Ctrl+N/O/S/Shift+S; project settings dialog; unsaved-changes prompt on New/Open/close window.

## Done when

- save → quit → open restores the identical project (round-trip test in `app-core` + manual check)
- changing frame rate or resolution is undoable and does not corrupt clip times (flicks stay exact)
- closing with unsaved changes asks Save / Don't save / Cancel
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
