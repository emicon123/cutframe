# Task 0240: Missing media detection and relink

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0230`, `0100`
**Spec:** `docs/design/media-import.md` (§6.6, §12 task 0240), `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `CLAUDE.md`

## Why

Projects break when media moves; relink is expected in every editor.

## Features

- [ ] `F0.25` Detect missing media on open; relink dialog (single file and whole folder) — *missing*

## Scope

- **backend:** `MediaStatus` (online/offline/changed) as session state in `AppState`; on open a background `checkingMedia` job resolves relative-then-absolute paths (ADR-0002 §7) and compares fingerprints; runtime offline detection on open/decode failure.
- **backend:** `RelinkMedia` (undoable), `relink_media(id, path, allow_mismatch)` and `relink_media_folder(folder)` as jobs; folder relink matches by fingerprint only (same name tried first) and is one `Batch` undo step (design §6.6).
- **frontend:** relink dialog (one file, whole folder, mismatch confirmation); offline/changed badges in the media panel. The offline placeholder on timeline clips is task 0260 and in the preview task 0320.

## Done when

- moving the media folder and relinking it restores the project with one action
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
