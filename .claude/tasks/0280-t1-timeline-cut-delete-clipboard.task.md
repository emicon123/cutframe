# Task 0280: Split, delete, ripple and clipboard

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0270`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

The cut-and-tighten loop that defines a quick editor.

## Features

- [ ] `F1.41` Split at playhead (Ctrl+B) — selected clip or all tracks — *missing*
- [ ] `F1.42` Delete clip (leaves gap on overlay tracks) — *missing*
- [ ] `F1.43` Ripple delete (closes the gap) — *missing*
- [ ] `F1.44` Magnetic main track (no gaps; clips reorder by drag) with on/off toggle — *missing*
- [ ] `F1.46` Copy / cut / paste / duplicate clips — *missing*
- [ ] `F1.47` Trim to playhead: delete left / delete right of playhead (Q / W) — *missing*
- [ ] `F1.48` Select and delete a gap — *missing*

## Scope

- **backend:** split at playhead (selected or all tracks), delete, ripple delete, delete gap, trim-to-playhead (Q/W), magnetic main track behaviour with a toggle, copy/cut/paste/duplicate.
- **frontend:** shortcuts (Ctrl+B, Delete, Shift+Delete, Q, W, Ctrl+C/X/V/D) registered in the shortcut registry; toolbar buttons.

## Done when

- property tests in `app-core`: no operation leaves overlapping clips or gaps on a magnetic main track
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
