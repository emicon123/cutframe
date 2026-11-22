# Task 0560: PiP, blend modes, masks, chroma key, canvas presets

**Tier:** 2 · **Agents:** `backend`, `frontend` · **Depends on:** `0520`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Layering tools that make overlays and green-screen edits possible.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.62` Picture-in-picture overlays on overlay tracks — *?*
- [ ] `F2.63` Blend modes: normal, darken, multiply, colour burn, lighten, screen, colour dodge, add, overlay, soft light, hard light, difference, exclusion — *?*
- [ ] `F2.64` Masks: linear, mirror (band), circle, rectangle with rounded corners, heart, star — *?*
- [ ] `F2.65` Mask position, size, rotation, feather, invert — *?*
- [ ] `F2.66` Chroma key (colour picker, intensity, edge/shadow, spill) — *?*
- [ ] `F2.67` Images with transparency (PNG, WebP) composite correctly — *?*
- [ ] `F2.68` Video with alpha channel (ProRes 4444, VP9 alpha WebM) — *?*
- [ ] `F2.69` Split-screen layout presets — *?*
- [ ] `F2.70` Canvas aspect ratio presets: 16:9, 9:16, 1:1, 4:3, 3:4, 4:5, 21:9, 2.35:1, custom — *?*
- [ ] `F2.71` Preview guides: safe areas, rule-of-thirds grid, centre lines, social-app UI overlay — *?*

## Scope

- **backend:** blend modes, shape masks with feather/invert, chroma key with spill suppression, alpha from PNG/WebP/ProRes 4444/VP9 alpha, split-screen presets, aspect ratio presets in project settings.
- **frontend:** mask handles in the preview, chroma colour picker, preview guides (safe areas, thirds, centre, social UI overlay).

## Done when

- golden-frame tests for each blend mode and mask shape
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
