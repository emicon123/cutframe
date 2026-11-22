# Task 0310: Design: render graph, playback and audio transport

**Tier:** 1 · **Agents:** `architect` · **Depends on:** `0250`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Architecture rules 2–4 and review F16/F17: the engine has no project input, no clock and no audio yet.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- design doc `docs/design/render-graph.md`: `render(snapshot, Time)` shared by preview and export, layer compositing order, colour pipeline baseline (8-bit vs 16-bit float, see F3.30/F3.33), frame cache, render on change vs continuous (review F14).
- design doc `docs/design/playback.md`: Rust-owned transport, audio as master clock, `cpal` output thread with `rtrb` (no allocations/locks/I/O), mixer, A/V sync, dropped frames, `playbackTick` events replacing the UI-only playhead.
- decide zero-copy decode for D3D12 (D3D12VA on the shared device vs D3D11VA shared handles, review F16), with the spike input in `docs/design/media-import.md` §5; the decoder already returns `FrameData::Cpu` with room for a GPU variant.
- accept or amend ADR-0004 and ADR-0005 if the design changes them; golden-frame test harness plan (review F44).

## Done when

- design docs reviewed; tasks 0320–0370 adjusted
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
