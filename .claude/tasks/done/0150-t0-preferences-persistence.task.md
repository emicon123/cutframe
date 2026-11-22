# Task 0150: Preferences and persisted layout

**Tier:** 0 · **Agents:** `backend`, `frontend` · **Depends on:** `0130`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Panel sizes, window size and editing defaults reset on every start.

## Features

- [ ] `F0.02` Resizable, collapsible panels; layout persisted between sessions — *partial*
- [ ] `F0.03` Window size/position/maximised state persisted — *missing*
- [ ] `F0.32` Preferences dialog (general, editing defaults, performance, cache, shortcuts) — *missing*
- [ ] `F0.33` Default image duration, default transition duration — *missing*
- [ ] `F0.34` Cache location and size; "clear cache" — *missing*
- [ ] `F0.35` UI language setting (English first, Polish later) — *missing*

## Scope

- **backend:** preferences file in app data with a schema version (same migration approach as ADR-0002); commands get/set; defaults for image duration and transition duration (used by later tasks).
- **backend:** cache directory setting with size report and clear-cache command (thumbnails, waveforms, proxies arrive later; wire the folder now).
- **frontend:** preferences dialog (General, Editing, Performance, Cache, Language); panels collapsible and their sizes persisted; window size/position/maximised restored (Tauri window-state plugin is fine).
- **frontend:** language setting wired to `i18n` (English only for now, F3.74 adds Polish).

## Done when

- restart restores window, panel layout and preferences
- preferences file from an older schema loads (migration test)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)

## Closing note (2026-11-22)

Moved to done after `pnpm check` and `cargo deny` passed on Windows. **The manual check in `pnpm dev` was not done**, and neither was the installer CI run (file association, single instance). The "manual check pending" notes in `docs/feature-catalog.md` still apply.
