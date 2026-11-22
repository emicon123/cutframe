# Task 0840: Polish locale and accessibility

**Tier:** 3 · **Agents:** `frontend` · **Depends on:** `0150`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

CLAUDE.md: Polish UI later; keyboard and screen-reader use.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.74` Polish UI locale — *?*
- [ ] `F3.75` Keyboard-only operation of all panels — *?*
- [ ] `F3.76` Screen-reader labels; high-contrast / UI scale options — *?*

## Scope

- **frontend:** Polish locale for all strings in `i18n`, plural forms; keyboard-only navigation of every panel; ARIA labels, UI scale, high-contrast option.

## Done when

- every UI string exists in both locales (test fails on missing keys)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
