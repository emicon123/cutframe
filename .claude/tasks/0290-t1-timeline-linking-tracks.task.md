# Task 0290: Linked audio, replace clip, track controls

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0280`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Video with sound must stay in sync while editing; tracks need mute/hide/lock.

## Features

- [ ] `F1.49` Linked video + audio of the same source move/trim together; link on/off toggle — *missing*
- [ ] `F1.50` Detach audio from a video clip onto its own track — *missing*
- [ ] `F1.51` Replace clip media while keeping position, duration and effects — *missing*
- [ ] `F1.52` Track controls: mute, hide, lock — *partial*
- [ ] `F1.53` Reorder tracks — *missing*
- [ ] `F1.56` Right-click context menu on clips and tracks — *missing*

## Scope

- **backend:** linked A/V groups move/trim together with a link toggle; detach audio; replace clip media keeping position/duration/effects; track mute/hide/lock commands (fields exist in the model); reorder tracks.
- **frontend:** track header buttons, drag to reorder tracks, right-click context menus on clips and tracks.

## Done when

- locked tracks reject edits; muted/hidden tracks are excluded from preview and export (verified once playback lands)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
