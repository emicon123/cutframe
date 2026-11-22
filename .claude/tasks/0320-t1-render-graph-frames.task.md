# Task 0320: Render the timeline frame at the playhead

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0310`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The preview shows a test pattern; it must show the composed timeline.

## Features

- [ ] `F3.45` Hardware-accelerated decoding (D3D11VA / D3D12VA on Windows) — *missing* (moved to Tier 1 on 2026-11-22)
- [ ] `F3.31` HDR sources (PQ/HLG, Rec.2020) with tone mapping to SDR — *missing* (moved to Tier 1 on 2026-11-22)

## Scope

- **backend:** engine depends on `app-core`, reads the `ArcSwap` snapshot, decodes the needed frames and composites video/overlay/image tracks at the playhead; render on change only (review F14); golden-frame tests with `insta` (review F44).
- **backend:** apply each source's `rotation`, `sampleAspect`, `origin` and colour info from the media model; timestamp-based frame selection (sample and hold, VFR-safe); an offline placeholder frame for clips whose media is offline (`docs/design/media-import.md` §5, §6.6).
- **backend:** hardware decode (D3D11VA/D3D12VA for HEVC, H.264, AV1) on the zero-copy path chosen in 0310, with software fallback per stream when the profile is unsupported or the hardware decoder fails; 10-bit sources stay 10-bit up to the tone map.
- **backend:** HDR→SDR tone map (HLG, PQ; simple operator per 0310) in the shared render graph, so preview and export match; golden frames for an SDR, an HLG and a PQ fixture.
- **backend:** the native preview and offscreen targets (`--smoke-test`, golden frames) use the same graph (ADR-0005 §6). The legacy Linux frames mode is out of scope (ADR-0005 amendment 2026-11-22).

## Done when

- scrubbing shows the correct frame in the native preview window on Windows, including on an integrated GPU and on WARP; golden-frame tests run locally (and in a manual CI run)
- an iPhone HDR (HLG / Dolby Vision 8.4) clip shows natural SDR colour, not grey or clipped, in the preview and in an offscreen render of the same frame
- with the hardware decoder disabled (forced software fallback) every fixture still renders
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
