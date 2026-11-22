# Task 0710: Auto captions and subtitle files

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0700`, `0590`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Auto captions are one of the most used features in short-form editors.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.01` Auto captions: local speech-to-text with language selection — *?*
- [ ] `F3.02` Caption editor: list of lines, edit text, timing, split/merge lines — *?*
- [ ] `F3.03` Batch styling of all captions; caption style presets — *?*
- [ ] `F3.04` Word-level highlight (karaoke-style) captions — *?*
- [ ] `F3.05` Import subtitles (SRT, ASS, LRC) — *?*
- [ ] `F3.06` Export subtitles (SRT, TXT) and burn-in on export — *?*

## Scope

- **architect:** model ADR — local speech-to-text (e.g. whisper.cpp, MIT), model download vs bundle, size, CPU/GPU cost, languages incl. Polish, word timestamps.
- **backend:** transcription job, caption track model, SRT/ASS/LRC import, SRT/TXT export, burn-in via the text renderer.
- **frontend:** caption editor (list, edit, split/merge, timing), batch styling, karaoke word highlight.

## Done when

- a 5-minute Polish clip is captioned offline with editable, correctly timed lines
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
