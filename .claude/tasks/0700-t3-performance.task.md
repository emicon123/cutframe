# Task 0700: Proxies, render cache, job queue, budgets

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0390`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

4K phone footage and long timelines must stay smooth; several T3 features need a shared analysis job queue.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.46` Proxy media: generate, toggle, auto-use for 4K / heavy codecs — *?*
- [ ] `F3.47` Render cache: pre-render heavy timeline sections — *?*
- [ ] `F3.48` Background analysis jobs queue (waveforms, proxies, stabilisation, transcription) — *?*
- [ ] `F3.49` Smooth editing of long timelines (1h+, hundreds of clips) — *?*
- [ ] `F3.50` Memory and GPU budget management with graceful degradation — *?*

## Scope

- **architect:** proxy and render-cache design (storage, invalidation, export always uses originals).
- **backend:** proxy generation and toggle (hardware decode itself moved to Tier 1, task 0320, on 2026-11-22), render cache, priorities and per-kind limits on top of the job manager from 0220 (`docs/design/media-import.md` §6.1), memory/GPU budgets, benchmark with 1h / 500-clip project (review F41).
- **frontend:** proxy toggle, job list, cache indicators on the timeline.
- **backend (from 0320 measurements, `docs/design/render-graph.md` §13):** the 4K P010 convert with `f32` filtering takes 3.0 ms on the Iris Xe (budget 2 ms) and the offline 1080p frame with a CPU source 10.9 ms wall (budget 8 ms); decide whether to optimise (e.g. sampler filtering for 8-bit SDR sources, upload/read-back overlap) and add the `tracing` spans to the present-rate log.

## Done when

- 4K HEVC phone footage scrubs smoothly on an integrated GPU with proxies on
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
