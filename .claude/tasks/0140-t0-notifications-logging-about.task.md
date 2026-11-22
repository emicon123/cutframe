# Task 0140: Notifications, log file and About/licences

**Tier:** 0 · **Agents:** `frontend`, `backend`, `devops` · **Depends on:** `0130`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Errors only show as one line in the status bar, logs go to stdout, and the LGPL notice required for FFmpeg has nowhere to live.

## Features

- [ ] `F0.08` Non-blocking notifications (toasts) and readable error dialogs — *partial*
- [ ] `F0.09` Logging to a file in app data; "open logs folder" — *partial*
- [ ] `F0.10` About dialog with version and third-party licence notices (FFmpeg LGPL notice, source offer) — *missing*

## Scope

- **frontend:** toast system for non-blocking messages and an error dialog for failures that need a decision; errors use the typed IPC error kinds (ADR-0003).
- **backend:** `tracing` also writes rolling log files to app data; command to open the logs folder.
- **frontend + devops:** About dialog with version, third-party licences (generated list of crates + npm packages) and the FFmpeg LGPL notice and source offer.

## Done when

- a failed command shows a toast with a readable message; the log file contains the details
- About lists licences generated at build time, not hand-written
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
