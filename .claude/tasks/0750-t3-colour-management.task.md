# Task 0750: 10-bit pipeline and better HDR tone mapping

**Tier:** 3 · **Agents:** `architect`, `backend` · **Depends on:** `0700`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Curves and HSL need a defined pipeline. Basic HDR→SDR tone mapping (F3.31) moved to Tier 1 (tasks 0310/0320, user decision 2026-11-22); this task may refine the operator once there is a 10-bit working pipeline.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.30` 10-bit (and higher) internal pipeline — *?*

## Scope

- **architect:** ADR on working colour space and bit depth of the render graph (export stays SDR Rec.709); revisit the Tier 1 tone-map operator only if it falls short.
- **backend:** implement per the ADR with golden-frame tests for Rec.709 and HLG/PQ sources.

## Done when

- curves and HSL on a 10-bit source show no banding that the 8-bit path shows
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
