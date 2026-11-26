# Task 0290: Linked audio, replace clip, track controls

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0280`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

> **Closed 2026-11-26 without the manual check (user's standing decision).** `pnpm check` passed on
> Linux (PR #9); the Windows `pnpm check` and the detach/link, replace media, track controls, track reorder and
> context-menu flows in `pnpm dev` were not done by hand. Preview/export do not honour mute/hide yet (task 0320).

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

Design: `docs/design/timeline-editing.md` (§3.3, §3.5, §4.12–4.16).

- **backend:** a video clip plays its own audio until it is detached; `detach_audio` creates a linked audio clip; `link_clips` / `unlink_clips` (the link toggle). Move/trim/split/delete link expansion already lands in 0270/0280; this task adds the commands that create and remove links. `replace_clip_media` keeping position/duration/props; `set_track_flag` (mute/hide/lock; `locked` exists since 0260 and every command already honours it: direct edits on locked tracks fail, and locked tracks never follow main-track ripples); `move_track` within its section (overlays / main / audio).
- **frontend:** track header buttons, drag to reorder tracks, right-click context menus on clips and tracks.

## Done when

- locked tracks reject edits; muted/hidden tracks are excluded from preview and export (verified once playback lands)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
