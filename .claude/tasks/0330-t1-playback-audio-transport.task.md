# Task 0330: Real-time playback with audio

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0320`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The heart of the MVP: press Space and watch the edit with sound.

## Features

- [ ] `F1.31` Auto-scroll timeline to follow the playhead during playback — *missing*
- [ ] `F1.58` Real-time playback of the composed timeline with audio/video sync — *missing*
- [ ] `F1.59` Play/pause (Space), current/total timecode display — *partial*
- [ ] `F1.60` Dropped-frame handling: audio is the master clock, video skips — *missing*
- [ ] `F1.67` Loop playback — *missing*
- [ ] `F1.68` J/K/L shuttle (reverse / pause / forward, repeated presses speed up) — *missing*
- [ ] `F1.69` Audio scrubbing while dragging the playhead — *missing*

## Scope

Design: `docs/design/playback.md` (§13 task breakdown), `docs/design/render-graph.md` §7; ADR-0004 and ADR-0005 amendments of 2026-11-22.

- **backend:** transport in Rust (`TransportShared`, state machine with preroll; play/pause/toggle, J/K/L rates ±1/2/4/8, step, loop over the whole timeline, end handling, pause snapping the playhead to the shown frame) and the commands of playback §8; `playback-tick` from the render thread through the engine event sink (≤ 60 Hz, last state always delivered).
- **backend:** `audio-feeder` thread owning the `cpal` stream (opened on play/scrub, closed 10 s after pause, device errors handled), one `rtrb` ring of fixed `AudioBlock`s (85 ms target fill), `OutputCallback` that only copies, converts channels and publishes the clock (architecture rule 4); `AudioClock` from cpal's playback timestamp with a `WallClock` fallback.
- **backend:** audio readers and `mix_block` (clip volume, linear fades, track mute) shared with export; shuttle rates other than 1x are muted.
- **backend:** vsync-paced playback: frame for `audible time + present lead`, dropped-frame skipping and counting, the lookahead scheduler (reader reuse across split points, A/B readers for jump cuts, reader limits, keyframe-only above 2x, catch-up), `pause_background` for background preview jobs while playing (`docs/design/media-import.md` §6.1).
- **backend:** audio scrubbing grains (80 ms, at most one per 40 ms).
- **frontend:** playhead follows `playbackTick` with seek generations (replace the UI-only `playheadStore` source), timeline auto-scroll, Space/J/K/L/arrows/Home/End, Play button state, loop toggle, audio scrubbing via `seek(..., scrub)` while dragging; optional dropped-frame indicator.

## Done when

- 1080p30 two-track edit plays in sync for 10 minutes without drift on a mid-range Windows laptop (|A/V offset| ≤ 1 frame, 0 dropped frames, 0 underruns; `docs/design/playback.md` §11)
- a 4K30 HEVC 10-bit HLG phone clip plus one overlay plays with 0 dropped frames over 60 s at Full quality on the iGPU with hardware decode, tone-mapped (F3.45 and F3.31 from task 0320); 4K60 stays within the §11 budget
- the audio callback passes a no-allocation check in tests (counting allocator), and the clock/scheduling simulation tests of playback §12 pass
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
