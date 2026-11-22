# Task 0610: Audio editing basics and voiceover

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0520`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Music and voice are half of a short-form video.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.76` Background music on audio tracks — *?*
- [ ] `F2.77` Extract audio from a video file into the media panel — *?*
- [ ] `F2.78` Voiceover recording from a microphone (device picker, level meter, countdown) — *?*
- [ ] `F2.79` Fade handles directly on timeline clips — *?*
- [ ] `F2.80` Volume line on the clip (rubber band) with keyframes — *?*
- [ ] `F2.81` Track volume and master volume — *?*
- [ ] `F2.82` Audio level meters (master, per track) — *?*
- [ ] `F2.83` Pan / balance; mono/stereo handling — *?*
- [ ] `F2.84` Audio speed change with pitch options — *?*
- [ ] `F2.85` Manual beat markers on music clips — *?*

## Scope

- **backend:** extract audio to a media asset, `cpal` input for voiceover recording (device list, levels, countdown), track and master volume, pan, level meters (computed off the audio thread from a lock-free tap), audio speed with pitch options, manual beat markers.
- **frontend:** fade handles and volume rubber band on clips, meters, record dialog.

## Done when

- recording a voiceover while the timeline plays lands in sync on a new audio track
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
