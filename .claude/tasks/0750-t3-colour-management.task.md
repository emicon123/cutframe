# Task 0750: Colour management, 10-bit and HDR

**Tier:** 3 · **Agents:** `architect`, `backend` · **Depends on:** `0700`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Phone HDR footage looks wrong without tone mapping; pro colour tools need a defined pipeline.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.30` 10-bit (and higher) internal pipeline — *?*
- [ ] `F3.31` HDR sources (PQ/HLG, Rec.2020) with tone mapping to SDR — *?*
- [ ] `F3.32` HDR export — *?*
- [ ] `F3.33` Colour-space aware pipeline (Rec.709, sRGB, Display P3) — *?*

## Scope

- **architect:** ADR on working colour space, bit depth of the render graph, HDR input handling and tone mapping, HDR export options.
- **backend:** implement per the ADR with golden-frame tests for Rec.709, Display P3 and HLG/PQ sources.

## Done when

- an iPhone HDR clip previews and exports to SDR without clipped or washed-out colour
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
