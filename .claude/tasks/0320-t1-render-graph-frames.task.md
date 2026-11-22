# Task 0320: Render the timeline frame at the playhead

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0310`, `0260`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The preview shows a test pattern; it must show the composed timeline.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- **backend:** engine depends on `app-core`, reads the `ArcSwap` snapshot, decodes the needed frames and composites video/overlay/image tracks at the playhead; render on change only (review F14); golden-frame tests with `insta` (review F44).
- **backend:** frames mode on Linux uses the same graph (ADR-0005).

## Done when

- scrubbing shows the correct frame on Windows (native window) and on the Pi (frames mode); golden-frame tests in CI
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
