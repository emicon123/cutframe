# Task 0590: Rich text, curved text, text presets and animations

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0510`, `0580`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Advanced titling on top of text basics.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.14` Per-character / per-word styling (rich text) — *?*
- [ ] `F2.15` Curved / arc text — *?*
- [ ] `F2.16` Text style presets (one-click styles) — *?*
- [ ] `F2.17` Save own text style preset — *?*
- [ ] `F2.18` Text animations: in, out, loop, with duration — *?*
- [ ] `F2.19` Text templates (styled, animated title compositions) — *?*
- [ ] `F2.20` Typewriter / per-character reveal animations — *?*

## Scope

- **backend:** per-character styling, arc text, text style presets and user presets, text in/out/loop and typewriter animations.
- **user:** decide whether bundled text templates are in scope (content licensing); if not, mark F2.19 `n/a`.

## Done when

- presets round-trip through save/load
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
