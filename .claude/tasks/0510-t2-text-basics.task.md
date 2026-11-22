# Task 0510: Text clips and styling

**Tier:** 2 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0500`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Titles and captions are the most used creative feature in short-form editing.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F2.01` Add a default text clip on a text track — *?*
- [ ] `F2.02` Edit text in the inspector and inline in the preview — *?*
- [ ] `F2.03` Multi-line text, text box with auto-wrap — *?*
- [ ] `F2.04` Font family from system fonts and bundled fonts — *?*
- [ ] `F2.05` Import custom font files — *?*
- [ ] `F2.06` Size, colour, bold, italic, underline, case — *?*
- [ ] `F2.07` Alignment, letter spacing, line spacing — *?*
- [ ] `F2.08` Stroke / outline (colour, width) — *?*
- [ ] `F2.09` Shadow (colour, opacity, blur, distance, angle) — *?*
- [ ] `F2.10` Background box (colour, opacity, padding, corner radius) — *?*
- [ ] `F2.11` Glow — *?*
- [ ] `F2.12` Gradient fill — *?*
- [ ] `F2.13` Emoji and full Unicode (RTL, CJK, Polish diacritics) — *?*

## Scope

- **architect (short ADR):** text shaping and rasterisation stack (e.g. a permissive Rust shaping crate) rendering on the GPU inside the render graph; system font enumeration on Windows.
- **backend:** text clip model and rendering with all styling properties; bundled fonts only with permissive licences (OFL is fine).
- **frontend:** add text, inspector styling panel, inline editing in the preview.

## Done when

- Polish diacritics, emoji, CJK and RTL render correctly in preview and export (golden frames)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
