# Task 0380: Design: export pipeline and encoder selection

**Tier:** 1 · **Agents:** `architect` · **Depends on:** `0310`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Export must reuse the render graph (rule 3) and stay LGPL-only (rule 7); review F28 (OpenH264 fallback).

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- design doc `docs/design/export.md`: offline render loop on the same graph, encoder priority (NVENC / Quick Sync / AMF / Media Foundation / OpenH264), container muxing, audio encode (FFmpeg native AAC), export as a background job with progress/cancel, error taxonomy.

## Done when

- design doc reviewed; tasks 0390–0400 adjusted
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
