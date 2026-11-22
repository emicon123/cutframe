# Task 0730: Loudness, noise reduction, ducking, voice effects, mixer

**Tier:** 3 · **Agents:** `backend`, `frontend` · **Depends on:** `0610`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Clean, consistent sound for voice-led videos.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.13` Loudness normalisation to a target (LUFS) — *?*
- [ ] `F3.14` Noise reduction — *?*
- [ ] `F3.15` Voice enhancement — *?*
- [ ] `F3.16` Automatic ducking (music lowers under voice) — *?*
- [ ] `F3.17` Voice changer / voice effects (pitch, robot, echo, reverb, telephone, chipmunk, monster) — *?*
- [ ] `F3.18` Equaliser — *?*
- [ ] `F3.19` Compressor / limiter — *?*
- [ ] `F3.22` Audio fade curve shapes — *?*
- [ ] `F3.24` Audio mixer panel — *?*

## Scope

- **backend:** real-time-safe DSP chain per clip/track (EQ, compressor/limiter, voice effects, fade curves), offline analysis jobs (LUFS normalisation, noise profile), auto ducking from voice detection; all DSP crates must be permissively licensed.
- **frontend:** audio effects panel and a mixer panel.

## Done when

- DSP on the audio thread allocates nothing (test); export loudness matches the target within 1 LU
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
