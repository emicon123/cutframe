# Task 0320: Render the timeline frame at the playhead

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0310`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The preview shows a test pattern; it must show the composed timeline.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- **backend:** engine depends on `app-core`, reads the `ArcSwap` snapshot, decodes the needed frames and composites video/overlay/image tracks at the playhead; render on change only (review F14); golden-frame tests with `insta` (review F44).
- **backend:** apply each source's `rotation`, `sampleAspect`, `origin` and colour info from the media model; timestamp-based frame selection (sample and hold, VFR-safe); an offline placeholder frame for clips whose media is offline (`docs/design/media-import.md` §5, §6.6).
- **backend:** the native preview and offscreen targets (`--smoke-test`, golden frames) use the same graph (ADR-0005 §6). The legacy Linux frames mode is out of scope (ADR-0005 amendment 2026-11-22).

## Done when

- scrubbing shows the correct frame in the native preview window on Windows, including on an integrated GPU and on WARP; golden-frame tests run locally (and in a manual CI run)
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
