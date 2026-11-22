# Task 0170: Auto-update and code signing

**Tier:** 0 · **Agents:** **user**, `devops`, `backend`, `frontend` · **Depends on:** `0160`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Hard constraint: the app must update itself. Review items F31–F33.

## Features

- [ ] `F0.12` Automatic updates (check, download, install, restart) — *missing*

## Scope

- **user:** freeze the product name, identifier and updater key (review F33, CLAUDE.md H5) — changing them after the first public release breaks updates. This task may wait for that decision; later tasks do not depend on it.
- **devops:** `tauri-plugin-updater` config, `latest.json` on GitHub Releases, signing keys in Actions secrets; Authenticode signing of the installer (review F32).
- **backend:** register the plugin; **frontend:** update prompt (download, progress, restart).

## Done when

- an installed older version updates to a newer draft release end-to-end, without admin rights
- no key or token in the repository
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
