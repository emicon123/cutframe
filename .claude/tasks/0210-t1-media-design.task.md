# Task 0210: Design: media assets, import, decode and media:// assets

**Tier:** 1 · **Agents:** `architect` · **Depends on:** `0200`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Media import touches the project model, IPC, jobs, threading and the `media://` protocol (CLAUDE.md: cross-cutting → architect first). Review P1 #1.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- design doc `docs/design/media-import.md`: `MediaId` and media assets in the project model (path, probe data, fingerprint for relink), import as a long-running job (`JobId` + progress, ADR-0003), probing, rotation/VFR/PAR handling, decode threads (ADR-0004).
- `media://` routes for thumbnails, filmstrips and waveforms by id, the bounded worker pool (review F12) and the disk cache layout.
- zero-copy decode choice for D3D12 (D3D12VA vs D3D11 shared handles, review F16) if it is decided here rather than in the render-graph design.
- project-file migration for the new fields (ADR-0002) and the test plan; split the implementation into the following tasks if this split is wrong.

## Done when

- design doc reviewed by the user; follow-up tasks 0220–0240 adjusted to it
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
