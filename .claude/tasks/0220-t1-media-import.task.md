# Task 0220: Import and probe media

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0210`
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

First step of the MVP: get video, audio and images into the project.

## Features

- [ ] `F1.01` Import video, audio and images via file dialog (multi-select) — *missing*
- [ ] `F1.02` Drag and drop files from the OS into the media panel — *missing*
- [ ] `F1.04` Video containers: MP4, MOV, MKV, WebM, AVI, M4V — *missing*
- [ ] `F1.05` Video codecs (decode): H.264, HEVC, VP8/VP9, AV1, ProRes, MPEG-2/4, MJPEG — *missing*
- [ ] `F1.06` Audio formats: MP3, WAV, AAC/M4A, FLAC, OGG/Opus — *missing*
- [ ] `F1.07` Image formats: JPG, PNG, WebP, BMP, TIFF, GIF (animated) — *missing*
- [ ] `F1.08` HEIC/HEIF photos (phone pictures) — *missing*
- [ ] `F1.09` Media probing: duration, resolution, frame rate, codec, audio channels/sample rate — *missing*
- [ ] `F1.10` Rotation metadata honoured (portrait phone videos) — *missing*
- [ ] `F1.11` Variable frame rate sources play and cut correctly — *missing*
- [ ] `F1.12` Non-square pixel aspect ratio and anamorphic sources displayed correctly — *missing*
- [ ] `F1.13` Corrupt / unsupported files rejected with a clear message — *missing*

## Scope

- **backend:** `import_media(paths)` job per the design: probe, add assets (undoable), report progress and per-file errors.
- **backend:** format coverage tests with tiny fixture files per container/codec (keep fixtures small and licence-clean).
- **frontend:** Import button + file dialog, drag and drop from Explorer onto the media panel; clear message for unsupported/corrupt files.

## Done when

- importing a mixed folder of phone videos, MP3s and photos works; portrait videos show upright
- HEIC either works or is reported as unsupported with a clear message (record which in the catalog)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
