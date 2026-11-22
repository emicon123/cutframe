# Task 0320: Render the timeline frame at the playhead

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0310`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The preview shows a test pattern; it must show the composed timeline.

## Features

- [ ] `F3.45` Hardware-accelerated decoding (D3D11VA / D3D12VA on Windows) — *missing* (moved to Tier 1 on 2026-11-22)
- [ ] `F3.31` HDR sources (PQ/HLG, Rec.2020) with tone mapping to SDR — *missing* (moved to Tier 1 on 2026-11-22)

## Scope

Design: `docs/design/render-graph.md` (§14 task breakdown) and the `seek` part of `docs/design/playback.md` (§8, §13); ADR-0004 and ADR-0005 amendments of 2026-11-22.

- **backend (first):** the D3D12VA zero-copy spike on the shared device (render-graph §5.4) on the iGPU and the dGPU; record the result in the design's status line before building on it.
- **backend:** `Arc<GpuContext>` with the optional `NV12` / `P010` / `16BIT_NORM` features and a non-sRGB surface; `AppState` shares `Arc<ArcSwap<Project>>` with the engine and sends `SnapshotChanged` on publish; engine depends on `app-core`.
- **backend:** `build_plan(snapshot, t, render_size)` (pure, insta-snapshotted) and the `Compositor` (background colour, source convert pass, layer pass with fit, colour clips, offline/decode-error placeholder) at the playhead; layer order and centre sampling per render-graph §4; render on change only (review F14, render-graph §7); source-texture cache and last-canvas skip (§6).
- **backend:** apply each source's `rotation`, `sampleAspect`, `origin` and colour info from the media model in the convert pass; timestamp-based frame selection (sample and hold, VFR-safe); `MediaUnavailable` reporting for offline media (`docs/design/media-import.md` §5, §6.6).
- **backend:** video readers (seek rules, coalescing, progressive scrub, CPU frame layouts, images) and hardware decode (D3D12VA for HEVC, H.264, AV1) on the zero-copy path with the release rule, falling back per stream to hardware decode with download and then to software decode; `CUTFRAME_HWDECODE=off`; 10-bit sources stay 10-bit up to the tone map (render-graph §5.1).
- **backend:** HDR→SDR tone map (HLG, PQ; BT.2390 EETF to 203-nit SDR white, render-graph §5.2) in the convert pass of the shared graph, with `engine::color` reference functions that the shader is tested against.
- **backend:** `OfflineRenderer` (`FetchPolicy::Exact`, reused targets, review F48) as the entry point for export and tests; golden-frame harness on WARP (PNG references with tolerance, `CUTFRAME_UPDATE_GOLDEN`, insta for plans; review F44, render-graph §12); `--smoke-test` presents the compositor's output and the test pattern is removed. The legacy Linux frames mode is out of scope (ADR-0005 amendment 2026-11-22).
- **backend:** `TransportShared` (playhead, seek generation) and the `seek(time, mode)` command; while paused the readers decode the exact frame plus 2 ahead (playback §7). `pnpm bindings`.
- **frontend:** call `seek` when the UI playhead changes (`scrub` while dragging, `exact` on release); the UI still owns the playhead until 0330.

## Done when

- scrubbing shows the correct frame in the native preview window on Windows, including on an integrated GPU and on WARP; golden-frame tests run locally on WARP (and in a manual CI run)
- the zero-copy spike result is recorded in `docs/design/render-graph.md`; a 4K HEVC 10-bit clip is decoded in hardware without a CPU copy on the iGPU and the dGPU (or the recorded fallback applies)
- an iPhone HDR (HLG / Dolby Vision 8.4) clip shows natural SDR colour, not grey or clipped, in the preview and in an offscreen render of the same frame
- with the hardware decoder disabled (`CUTFRAME_HWDECODE=off`) every fixture still renders
- paused and idle, the preview presents nothing and the render thread is blocked
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
