# Task 0370: Volume, fades, speed, reverse, freeze frame

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0330`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Remaining basic clip properties for an MVP edit.

## Features

- [ ] `F1.77` Clip volume (percent / dB) — *missing*
- [ ] `F1.78` Audio fade in / fade out — *missing*
- [ ] `F1.79` Constant speed change (e.g. 0.1×–100×) with duration updated — *missing*
- [ ] `F1.80` Preserve pitch when changing speed (toggle) — *missing*
- [ ] `F1.81` Reverse clip — *missing*
- [ ] `F1.82` Freeze frame (insert a still of the current frame) — *missing*
- [ ] `F1.83` Default duration for still images; image duration change — *missing*

## Scope

- **design note** (see also `docs/design/playback.md` §6.3 and `docs/design/render-graph.md` §4.1: readers and `build_plan` only call `Clip::source_time`, so speed needs no render-graph change): volume and fades are already `AudioProps` (`volume`, `fadeIn`, `fadeOut`) since 0260 and are set with `set_clip_property`; speed and reverse are new clip fields with a schema migration and change only `Clip::source_time` / `source_end` (`docs/design/timeline-editing.md` §3.2).
- **backend:** clip volume and fades are already in `mix_block` from 0330 (`docs/design/playback.md` §6.3); this task adds constant speed with duration update and pitch-preserving time-stretch per voice on the audio feeder thread, never in the callback (permissive-licence library or own implementation; no GPL); reverse through a GOP-reverse reader mode (decode a GOP forward into a bounded CPU frame stack, emit it backwards; playback §13); freeze frame insert; image default duration from preferences.
- **frontend:** inspector controls and timeline speed badge.

## Done when

- 2× and 0.5× clips play in sync with pitch preserved when the toggle is on
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
