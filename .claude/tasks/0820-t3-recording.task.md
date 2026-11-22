# Task 0820: Screen, webcam and teleprompter recording

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0610`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Creating content directly inside the editor.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.62` Screen recording built into the app — *?*
- [ ] `F3.63` Webcam recording built into the app — *?*
- [ ] `F3.64` Teleprompter for voiceover/webcam recording — *?*

## Scope

- **architect:** capture APIs on Windows (Windows.Graphics.Capture, Media Foundation); encoding during capture.
- **backend/frontend:** recording dialogs; recordings land in the media panel; teleprompter overlay window.

## Done when

- a 10-minute screen + mic recording imports cleanly and stays in sync
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
