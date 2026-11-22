# Task 0520: Keyframes and easing

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0500`, `0350`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Keyframes unlock motion for every property; masks/filters later reuse it.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.53` Keyframes on transform and opacity — *?*
- [ ] `F2.54` Keyframes on volume — *?*
- [ ] `F2.55` Keyframes on filter intensity, adjustments and effect parameters — *?*
- [ ] `F2.56` Keyframes on mask and crop — *?*
- [ ] `F2.57` Add/remove keyframe at playhead; previous/next keyframe navigation — *?*
- [ ] `F2.58` Keyframe markers on the clip in the timeline; drag to retime — *?*
- [ ] `F2.59` Easing: linear, ease in, ease out, ease in-out, hold — *?*
- [ ] `F2.60` Custom Bézier easing / graph editor — *?*
- [ ] `F2.61` Motion path shown in the preview — *?*

## Scope

- **backend:** keyframe commands (add/remove/move, easing), evaluation in the render graph and the mixer (volume).
- **frontend:** keyframe toggle next to every animatable property, prev/next navigation, diamonds on clips with drag-to-retime, easing presets and a Bézier graph editor, motion path in the preview.
- F2.55/F2.56 apply as filters and masks land (tasks 0540, 0560): the mechanism must make them work without extra model changes.

## Done when

- animating position + opacity + volume renders identically in preview and export
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
