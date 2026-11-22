# Task 0390: Export to MP4 (H.264/HEVC + AAC)

**Tier:** 1 · **Agents:** `backend`, `frontend` · **Depends on:** `0380`, `0370` (and through it
`0320`/`0330` for `OfflineRenderer` and `mix_block`); the job manager of PR #2 (task 0220) must be
merged first
**Spec:** `docs/design/export.md` (the design for this task), `docs/design/render-graph.md` §4.5,
`docs/design/playback.md` §6.3, `docs/feature-catalog.md` (feature IDs below), `docs/adr/`,
`CLAUDE.md`

## Why

Last MVP step: get the edit out of the app.

## Features

- [ ] `F1.86` Export dialog: file name, destination folder — *missing*
- [ ] `F1.87` Resolution: 480p, 720p, 1080p, 2K, 4K (keeping project aspect) — *missing* (2K = 1440p; also "project size")
- [ ] `F1.88` Frame rate: 24, 25, 30, 50, 60 fps — *missing* (plus the project rate when it is ≤ 60)
- [ ] `F1.89` Bitrate: low / recommended / high / custom — *missing*
- [ ] `F1.90` H.264 export in MP4 — *missing*
- [ ] `F1.91` HEVC export in MP4/MOV — *missing* (MP4 only, hardware encoders only; MOV is dropped, see design Decisions)
- [ ] `F1.92` AAC audio in the export — *missing*
- [ ] `F1.93` Export uses the same render graph as preview — *missing*
- [ ] `F1.94` Progress with ETA, cancel, background export while editing — *missing*
- [ ] `F1.95` Hardware-accelerated encoding with software fallback — *missing*
- [ ] `F1.99` Clear error when export fails (disk full, encoder missing) — *missing*

## Scope

Follow `docs/design/export.md` §12. In short:

- **backend:**
  1. `jobs.rs`: `JobManager::start_dedicated`, `JobContext::set_stage`; `JobStage` keys
     `probingEncoders`, `exportPreparing`, `exportEncoding`, `exportFinalizing`; `IpcError::FileExists`
     and `IpcError::Busy` (design §5.1, §6).
  2. `engine::export::plan` (pure): output size, frame rate rules, frames and samples, YouTube
     bitrate table × quality, estimate; path rules (design §4.1, §5.2).
  3. `engine::export::encoders`: candidate order NVENC → QSV → AMF → MF hardware → MF software →
     OpenH264 (HEVC: the first four), `Full`/`Safe` option sets checked against the pinned build,
     the session probe (real test encodes), `CUTFRAME_ENCODERS`, `required_encoders_present`;
     `get_export_capabilities` + `export-capabilities-changed` (design §4.4).
  4. `engine::export::nv12`: BT.709 limited NV12 pass, left chroma siting, ring of 3 staging
     buffers, CPU copy (NV12, or I420 for OpenH264) (design §4.3).
  5. `engine::export::audio` + `mux`: AAC-LC 384 kbps from the shared `mix_block` at the project
     rate, MP4 with `+faststart`, colour tags bt709/limited, `hvc1` for HEVC (design §4.5, §4.6).
  6. `engine::export::run` + `output`: dedicated below-normal `export` thread, render loop on the
     snapshot, encoder fallback only before the first muxed packet, cancel, throttle during
     playback, free-space floor, partial file + rename + cleanup, error taxonomy (design §4.2,
     §4.7, §5).
  7. `check_export`, `start_export`, `export-report`, sleep prevention, `QuitRequested { unsaved,
     exporting }`; `pnpm bindings`; tell frontend what changed.
- **frontend:** export dialog (presets YouTube 1080p / YouTube 4K / Shorts, name, folder, custom
  section with resolution, fps, codec, quality, encoder), summary and issues from `check_export`,
  overwrite confirmation, preview hidden while the dialog is open; Ctrl+E and *File → Export…*;
  progress card with ETA (computed in the UI from `job-progress`) and Cancel, outside the preview;
  toasts per `ExportErrorKind`; quit confirmation while exporting (design §2, §7).
- **docs (by whoever updates the catalog rows):** F1.91 text becomes "HEVC export in MP4". The
  dialog's summary line shows the estimated file size, so update `F2.102` as well.

## Done when

- a 1-minute 1080p30 edit exports on a machine without a discrete GPU (Media Foundation software
  encoder or OpenH264) and plays in the Windows Media Player and VLC
- the WARP tests of design §11 pass in `cargo test --workspace` with `OpenH264`: NV12 parity,
  end-to-end golden export (exported frames vs the offline render of the same frames), container
  checks (`moov` first, bt709/limited tags, CFR, A/V length), A/V sync, cancel, offline media,
  encoder fallback, disk full
- the budgets of design §10 are measured on the reference laptop with the export timing log and
  the numbers are written into the design's status section (4K30 HLG → 4K H.264 via NVENC ≥ 1×
  real time; playback during a 1080p export drops no frames)
- every feature above works end-to-end in `pnpm dev` and has tests (`app-core` unit/property tests for commands, Vitest for UI logic, golden frames for rendering)
- each row above is updated in `docs/feature-catalog.md` (status + short evidence note); partial rows say what is missing
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
