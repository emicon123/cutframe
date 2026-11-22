# Task 0400: Audio-only export, frame snapshot, after-export actions

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0390`
**Spec:** `docs/design/export.md` (§6 `ExportTarget`, §12 "0400"), `docs/feature-catalog.md`
(feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Small export variants users expect from day one.

## Features

- [ ] `F1.96` Export audio only (WAV, MP3, AAC) — *missing*
- [ ] `F1.97` Export the current frame as an image (PNG/JPG) — *missing*
- [ ] `F1.98` Open the folder / play the file when the export finishes — *missing*

## Scope

All three reuse the 0390 export job, `check_export`/`start_export`, `export-report` and the file
rules of `docs/design/export.md` §5.2; no second job path.

- **backend:**
  - `ExportTarget::Audio { format: Wav | Mp3 | Aac }`: the same `mix_block` path at the project
    sample rate, no video. WAV `pcm_s16le`; MP3 `libmp3lame` 320 kbps CBR (in the pinned LGPL
    build); AAC-LC 256 kbps in `.m4a` (`ipod` muxer, `+faststart`). Extensions `.wav`, `.mp3`,
    `.m4a`.
  - `ExportTarget::Still { format: Png | Jpeg, time: Time }`: `OfflineRenderer::render_frame` with
    `OutputFormat::Rgba8` at the canvas size for the frame at `time` (the playhead), encoded with
    FFmpeg's `png` (RGB24) or `mjpeg` (`yuvj444p`, quality about 95) encoder; a short job on the
    pool, not the dedicated thread. Extensions `.png`, `.jpg`.
  - `reveal_exported_file(path)` and `open_exported_file(path)`: accepted only for paths produced
    by an export of this session (the webview cannot open arbitrary files); reuse the helper behind
    `reveal_media`.
  - `pnpm bindings`; tell frontend what changed.
- **frontend:** "Video / Audio only / Current frame" choice in the export dialog with the format
  options; *Show in folder* and *Play* on the done toast and the progress card.

## Done when

- all three work from the export dialog and are covered by tests (WAV/MP3/AAC decode back with
  the right length and a 1 kHz test tone at the right level; the PNG of a golden scene matches its
  golden reference; reveal/open reject a path that no export produced)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
