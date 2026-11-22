# Task 0720: Transcript editing, silence removal, TTS, translation

**Tier:** 3 · **Agents:** `architect`, `backend`, `frontend` · **Depends on:** `0710`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Speech-driven editing on top of the transcript.

## Features

These rows have not been audited yet (`?`). First check the code, set each row's status in the
catalog, and drop anything already `done` from the scope.

- [ ] `F3.07` Bilingual captions / translation — *?*
- [ ] `F3.08` Lyrics recognition on music — *?*
- [ ] `F3.09` Text-to-speech from a text clip — *?*
- [ ] `F3.10` Transcript-based editing: delete words in the transcript to cut the video — *?*
- [ ] `F3.11` Remove filler words — *?*
- [ ] `F3.12` Auto remove silences / pauses — *?*

## Scope

- **architect:** decide local vs online for translation, lyrics and TTS (privacy, licence, cost); some rows may become `n/a`.
- **backend/frontend:** transcript-based cutting (one `Batch`), filler-word and silence removal with review before applying, TTS clips (OS voices first), bilingual captions, lyrics recognition.

## Done when

- removing silences on a talking-head clip is one undo step and keeps A/V sync
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
