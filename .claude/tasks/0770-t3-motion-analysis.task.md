# Task 0770: Stabilisation, tracking, auto reframe, scene detection

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0700`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Analysis-driven features that share motion estimation.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.34` Video stabilisation (strength levels) — *?*
- [ ] `F3.38` Motion tracking: attach text/sticker/effect to a moving object — *?*
- [ ] `F3.39` Tracked privacy blur (follow a face/plate) — *?*
- [ ] `F3.40` Auto reframe to another aspect ratio following the subject — *?*
- [ ] `F3.41` Scene detection: auto split a clip at scene cuts — *?*

## Scope

- **architect:** design for analysis data storage (per media, cached) and which algorithms (classical CV first; models only with an ADR).
- **backend:** stabilisation, point/region tracking driving keyframes, tracked blur, subject-following reframe, scene-cut detection with split.
- **frontend:** tracking UI in the preview, analysis progress.

## Done when

- a tracked text follows a moving object across a 10-second clip without manual fixes
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
