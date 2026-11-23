# Task 0260: Put media on the timeline; main and overlay tracks

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0250`, `0230`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

> **Closed 2026-11-23 without the manual check (user's standing decision).** `pnpm check` passed on
> Windows and Linux (PR #5); the drag-and-drop flows in `pnpm dev` (media panel and Explorer drops, `+` button,
> drop over the native preview window, offline clips) were not done by hand.

## Why

Core MVP step: media panel → timeline.

## Features

- [ ] `F1.03` Drag and drop files from the OS directly onto the timeline — *missing*
- [ ] `F1.24` Timeline with a main video track — *partial*
- [ ] `F1.25` Add media to the timeline (drag, "+" button, append at end, insert at playhead) — *missing*
- [ ] `F1.26` Overlay video tracks above the main track (picture-in-picture) — *partial*
- [ ] `F1.27` Audio tracks below the main track — *partial*
- [ ] `F1.28` Tracks created automatically when dropping onto empty space; empty tracks removed — *partial*
- [ ] `F1.18` "Used in timeline" badge — *missing* (moved from 0230, `docs/design/media-import.md` §12)

## Scope

Design: `docs/design/timeline-editing.md` (§3 model, §4.1, §9 migration, §13 step 1).

- **backend (model first):** the clip/track model of §3: `ClipSource` (media + `sourceIn` / colour / text), `LinkId`, keyframe-ready `VisualProps`/`AudioProps` (`Animated<T>`), `Project.mainTrack`/`magnetic`/`background`, `Track.locked`; extended `Project::validate`; schema bump + migration + fixtures (§9; if the media migration landed first, this is the next step); view types and specta; shared helpers (`place_clip`, `repack_main`, `normalize_links`, `follow_main`: clips on unlocked non-main tracks follow main-track ripples via derived anchors, with bumping on collision, §3.7).
- **backend:** `add_clips(sources, placement)` (append / insert at playhead / drop on a track or a new track, one undo step); `add_track` clamps into the right section; the main track cannot be removed; emptied tracks are removed on commit; `RemoveClipsOfMedia` added to the remove-media batch from task 0230.
- **frontend:** drag from media panel and from Explorer onto the timeline with a drop preview (Explorer drop = import entry + add entry); `+` button on media items; main track drawn distinctly. Internal drags use pointer events, not HTML5 drag-and-drop (`docs/design/media-import.md` §2).
- **backend + frontend (from 0230/0240):** `MediaView.usedBy` counts clips and drives the used-in-timeline badge; clips of offline media draw an offline placeholder.

## Done when

- a short edit can be assembled from the media panel with drag and drop; every step undoes in one go (a drop from Explorer is the import step plus the add step)
- inserting on the main track shifts the overlays, text and audio after the insert point in the same undo step
- the v1 fixture loads through the migration chain and the new fixture round-trips
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
