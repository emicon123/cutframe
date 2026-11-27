# Task 0370: Volume, fades, speed, reverse, freeze frame

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0330`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Remaining basic clip properties for an MVP edit.

## Features

- [ ] `F1.77` Clip volume (percent / dB) — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.78` Audio fade in / fade out — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.79` Constant speed change (e.g. 0.1×–100×) with duration updated — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.80` Preserve pitch when changing speed (toggle) — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.81` Reverse clip — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.82` Freeze frame (insert a still of the current frame) — *partial* (built and tested; manual `pnpm dev` check open)
- [ ] `F1.83` Default duration for still images; image duration change — *partial* (built and tested; manual `pnpm dev` check open)

## Scope

**Design: `docs/design/clip-speed-audio.md`** (decisions, model, commands, IPC, engine, UI, migration v3 → v4, tests, task order). It supersedes the design note below where they differ: this task also builds `set_clip_property` for every `PropertyValue` kind (with gesture phases) and the selection-driven inspector shell (Audio, Speed, Duration sections; 0350 adds Transform); freeze frames are a `ClipSource::Freeze` kind, not an image file; the time-stretcher is FFmpeg `atempo` in a per-voice `libavfilter` graph.

- **design note** (see also `docs/design/playback.md` §6.3 and `docs/design/render-graph.md` §4.1: readers and `build_plan` only call `Clip::source_time`, so speed needs no render-graph change): volume and fades are already `AudioProps` (`volume`, `fadeIn`, `fadeOut`) since 0260 and are set with `set_clip_property`; speed and reverse are new clip fields with a schema migration and change only `Clip::source_time` / `source_end` (`docs/design/timeline-editing.md` §3.2).
- **backend:** clip volume and fades are already in `mix_block` from 0330 (`docs/design/playback.md` §6.3); this task adds constant speed with duration update and pitch-preserving time-stretch per voice on the audio feeder thread, never in the callback (permissive-licence library or own implementation; no GPL); reverse through a GOP-reverse reader mode (decode a GOP forward into a bounded CPU frame stack, emit it backwards; playback §13); freeze frame insert; image default duration from preferences.
- **frontend:** inspector controls and timeline speed badge.

## Done when

- 2× and 0.5× clips play in sync with pitch preserved when the toggle is on
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Status (2026-11-27, branch `claude/project-thread-z58xeb`)

Defaults used for the design's open questions: sound silent above 4×; freeze frames use the image
duration preference (5 s); preserve pitch on by default; a speed change that collides on a
non-magnetic track fails with `overlap`; `atempo` as the stretcher; 4K reverse preview best effort.

- [ ] **2× and 0.5× clips play in sync with pitch preserved:** mixer tests (1 kHz sine stays
  1000 Hz at 2× and 0.5×, audio within 5 ms of the clip mapping) and the Iris Xe playback harness
  (1080p30 at 2× and 0.5×, 0 % dropped, |A/V| 21 ms, 0 underruns). Open: listening in `pnpm dev`.
- [ ] **Every feature end-to-end in `pnpm dev` with tests:** app-core unit and property tests for
  every command and the mapping, migration fixtures, mixer/stage/sync/parity tests, reader and
  golden-frame tests (2×, 0.5×, reverse, freeze), Vitest for the inspector, badges and actions.
  Open: a person trying the inspector, speed, reverse, freeze frame and image duration in `pnpm dev`.
- [x] **Catalog rows updated:** F1.77–F1.83 (partial: manual check open).
- [x] **`pnpm check` passes; `pnpm bindings` committed.**
- [x] **Architecture rules:** every model change is an undoable command (schema v4 with
  migration); no frames over IPC; integer flicks in the mapping; time-stretch through FFmpeg's LGPL
  `avfilter` DLL, no new dependency.
- Known limits: 4K reverse preview drops 6–66 % of frames (re-decode and download bound; export is
  exact); `atempo` sounds rough on music outside about 0.75×–1.5× (upgrade path:
  `signalsmith-stretch`, MIT, documented in the design).
