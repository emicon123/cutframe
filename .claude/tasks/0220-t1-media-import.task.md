# Task 0220: Import and probe media

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0210`
**Spec:** `docs/design/media-import.md` (§3–§6, §12 task 0220), `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `CLAUDE.md`

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

- **backend:** `app-core` media model (`MediaId`, `MediaAsset`, `MediaInfo`, `Rational`, `Time::from_ticks`), `AddMedia` (`importMedia`), schema v1 → v2 migration + `project-v2.json` fixture, media rules for `Project::validate` (design §3, §8). Assets store `absolute` paths only; `relative` waits for task 0100.
- **backend:** `engine::media::probe` and `fingerprint` (BLAKE3 over size + first/last 1 MiB): rotation, pixel aspect, timestamp-based (VFR-safe) timing, HEIC tile grids or a clear `unsupportedFormat` (design §5).
- **backend:** the shared job manager (bounded pool, interactive/background queues, `cancel_job`, `job-progress`, `job-finished`, `IpcError::Cancelled`) and `import_media(paths)` / `import_media_dialog()`: per-file outcomes in `media-job-report`, duplicates skipped by fingerprint, all successes added as one undo step, nothing added on cancel (design §4, §6.1–6.2).
- **backend:** `MediaView` in `ProjectState`, constants `MEDIA_EXTENSIONS` etc. via `Builder::constant`; `cargo deny` for the new crates (`blake3`, `tauri-plugin-dialog`).
- **backend:** format coverage tests with tiny committed fixtures per container/codec (≤ 2 MB total, self-generated, licence-clean; design §10).
- **frontend:** Import button + Ctrl+I (dialog opened from Rust), drag and drop from Explorer onto the media panel via Tauri's drag-drop event (internal drags use pointer events, design §2), progress with cancel, result toast with translated per-file reasons, a plain list of imported items (the grid is 0230).

## Done when

- importing a mixed folder of phone videos, MP3s and photos works; portrait videos show upright
- HEIC either works or is reported as unsupported with a clear message (record which in the catalog)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
