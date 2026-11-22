# Task 0500: Design: effects-as-data, keyframes, track segments

**Tier:** 2 · **Agents:** `architect` · **Depends on:** `0400`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Rule 5 (effects are `effect.json` + `shader.wgsl`) and review F18. Filters, transitions, effects, text animations and adjustment layers all build on this.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- design doc `docs/design/effects.md`: `effect.json` schema (id, version, kind: filter/effect/transition/animation, typed parameters, defaults, UI hints), WGSL entry-point contract, packaging as Tauri resources (devops), project references by id + version (ADR-0002 §6).
- effect stacks on clips vs. effect/filter/adjustment segments on their own tracks (adjustment layer semantics), transition model with handles (F2.24).
- keyframe model: animated parameters, interpolation and easing, evaluation per frame in the render graph.
- library UX data needed by the UI (categories, preview thumbnails/animations).

## Done when

- design doc reviewed; T2 tasks adjusted
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
