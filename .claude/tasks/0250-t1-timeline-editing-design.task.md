# Task 0250: Design: clip model and timeline edit operations

**Tier:** 1 · **Agents:** `architect` · **Depends on:** `0210`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Clips today are only name/start/duration. Trims, linking, the magnetic main track and later keyframes all depend on how clips and properties are modelled.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- design doc `docs/design/timeline-editing.md`: clip → media reference with source in/out, image/text/colour clips, linked A/V groups, main (magnetic) track semantics vs overlay tracks, track lock.
- the list of edit commands (add, move, trim, split, delete, ripple delete, paste…) with their invariants, and which run as ADR-0006 gestures.
- where selection lives (UI vs model), snapping inputs, clipboard format.
- clip property model that is keyframe-ready (animated values, F2.53) so Tier 1 transform properties do not need a migration later.

## Done when

- design doc reviewed; tasks 0260–0300 and 0350–0370 adjusted to it
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
