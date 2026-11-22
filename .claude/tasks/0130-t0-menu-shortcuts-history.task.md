# Task 0130: Main menu, shortcuts and history panel

**Tier:** 0 · **Agents:** `frontend`, `backend` · **Depends on:** `0100`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Only undo/redo have shortcuts and there is no menu, so features added later have nowhere consistent to live.

## Features

- [ ] `F0.05` Main menu / command bar (File, Edit, View, Help) with shortcuts shown — *missing*
- [ ] `F0.06` Default keyboard shortcuts for all common actions — *partial*
- [ ] `F0.07` Keyboard shortcut reference panel — *missing*
- [ ] `F0.31` History panel (list of steps, jump back) — *missing*

## Scope

- **frontend:** menu / command bar (File, Edit, View, Help) driven by the single registry in `features/editor/shortcuts.ts`; every entry shows its shortcut.
- **frontend:** shortcut reference panel (`?` / Ctrl+/) generated from the registry; registry entries carry i18n labels.
- **backend:** command to list history entries (labels) and jump to an entry (repeated undo/redo in one call); **frontend:** history panel.

## Done when

- every action added from now on is registered once and appears in menu, shortcut panel and tooltips
- jumping to a history entry equals the same number of undo/redo steps (test)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
