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

- **backend:** transport in Rust (play/pause/seek/loop/JKL rates), `cpal` audio thread per architecture rule 4, mixer for audio tracks, video frames scheduled against the audio clock, dropped-frame skipping; decode workers use the hardware decode path from 0320 and pause background preview jobs during playback (`docs/design/media-import.md` §6.1).
- **frontend:** playhead follows `playbackTick` (replace the UI-only `playheadStore` source), timeline auto-scroll, Space/J/K/L, loop toggle, audio scrubbing while dragging.

## Done when

- 1080p30 two-track edit plays in sync for 10 minutes without drift on a mid-range Windows laptop
- a 4K30 HEVC 10-bit HDR phone clip plays in real time with hardware decode on the reference iGPU laptop (dropped frames counted and below 1 %), tone-mapped (F3.45 and F3.31 from task 0320)
- audio thread passes a no-allocation check in tests
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
