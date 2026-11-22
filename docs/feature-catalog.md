# Feature catalog

The full list of features a modern consumer desktop video editor (the product category Cutframe
targets) is expected to have, ordered from the most fundamental to the most optional. We use it to
audit what already exists and to pick the next work items.

## How to use this list

- Work top to bottom. A tier should be essentially complete before the next one gets serious
  attention; within a tier, rows are ordered roughly by dependency and importance.
- Every row has a stable ID (`F<tier>.<nn>`). Reference it in task files, design docs and commit
  messages (`feat(timeline): ripple delete (F1.43)`). Do not renumber; add new rows at the end of a
  section with the next free number in that tier, or with a suffix (`F1.43a`).
- Update the **Status** column during audits:

| Status | Meaning |
|---|---|
| `?` | not checked yet |
| `done` | implemented, tested, usable end-to-end in the app |
| `partial` | some of it exists (say what is missing in Notes) |
| `missing` | checked, nothing there |
| `n/a` | deliberately out of scope (say why in Notes) |

- **Notes** flag architecture or licence constraints. The most common ones:
  - *LGPL:* FFmpeg is LGPL-only (`CLAUDE.md`). GPL encoders such as `libx264`/`libx265` are not
    allowed; H.264/HEVC encoding must use hardware or OS encoders (NVENC, Quick Sync, AMF, Media
    Foundation) or a permissively licensed software encoder. The bundled OpenH264 is excluded:
    Cisco's patent licence covers only Cisco's own binaries downloaded at run time (review F28).
    Selection lives in `engine::media::encoders` (allowlist plus a hard GPL/nonfree denylist).
  - *Model:* needs an ML model; licence, model size, CPU/GPU cost and offline use need an ADR.
  - *Content:* needs a library of licensed assets (music, stickers, stock footage, templates); that is
    a product/legal decision, not just code.
  - *ADR:* cross-cutting; starts with the `architect` agent.

## Work plan

Every feature of tiers 0–3 that is not `done` or `n/a` belongs to exactly one task file in `.claude/tasks/`:
`01xx` tier 0, `02xx`–`04xx` tier 1, `05xx`–`06xx` tier 2, `07xx`–`08xx` tier 3. Design tasks for the
`architect` come before the feature tasks they unblock. Numbers step by 10, so new tasks can be
inserted between existing ones.

## Audit log

| Date | Commit | Scope | Result |
|---|---|---|---|
| 2026-11-21 | `2c0c2f7` | Tiers 0–1 | Tier 0: 6 done, 10 partial, 19 missing. Tier 1: 4 done, 9 partial, 86 missing (no media, clip editing, playback or export yet). |
| 2026-11-22 | `f37e796` | F0.28, F0.29 | Done by task 0006 (edit transactions). Tier 0 now: 8 done, 10 partial, 17 missing. |
| 2026-11-22 | `3841d6f` | F3.31, F3.45 | User decision on the media design: basic HDR→SDR tone mapping and hardware decode move to Tier 1 (Preview section, IDs kept; tasks 0310/0320/0330). Task 0700 and 0750 trimmed. |
| 2026-11-22 | `8174ada` | Tiers 2–3 | Private hobby use (`CLAUDE.md` "Purpose"): 35 rows set to `n/a`; tasks 0620, 0790, 0820 and 0830 removed, others trimmed. |

## Tiers

| Tier | Name | Meaning |
|---|---|---|
| 0 | Foundation | App shell, projects, undo/redo, settings. Nothing else works without it. |
| 1 | Core editing (MVP) | Import → arrange/cut on a timeline → preview → export. The smallest useful editor. |
| 2 | Essential creative tools | Text, transitions, filters, effects, keyframes, masks, audio basics. What users expect on day one. |
| 3 | Advanced / pro | Captions, pro audio and colour, tracking, stabilisation, performance features, interchange. |
| 4 | Smart / AI features | Features that need ML models (segmentation, generation, enhancement). |
| 5 | Ecosystem and online | Cloud, accounts, asset stores, templates marketplace, publishing. Mostly needs product decisions. |

---

## Tier 0 — Foundation

### App shell

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.01 | App starts into a main editor window with the standard layout: media/library panel, preview, inspector (properties), timeline | `EditorLayout.tsx`: top bar, media bin, preview, inspector, timeline, status bar | done |
| F0.02 | Resizable, collapsible panels; layout persisted between sessions | Resizable via `ResizablePanelGroup`; not collapsible, sizes not persisted | partial |
| F0.03 | Window size/position/maximised state persisted | | missing |
| F0.04 | Dark theme (default), consistent design system | `index.html` `class="dark"`, Tailwind tokens + shadcn/ui | done |
| F0.05 | Main menu / command bar (File, Edit, View, Help) with shortcuts shown | Top bar has only undo/redo and a disabled Export | missing |
| F0.06 | Default keyboard shortcuts for all common actions | Only undo/redo (`shortcuts.ts`); registry is ready for more | partial |
| F0.07 | Keyboard shortcut reference panel | | missing |
| F0.08 | Non-blocking notifications (toasts) and readable error dialogs | Last error shown in the status bar; no toasts or dialogs | partial |
| F0.09 | Logging to a file in app data; "open logs folder" | `tracing` logs to stdout only; no file, no "open logs folder" | partial |
| F0.10 | About dialog with version and third-party licence notices (FFmpeg LGPL notice, source offer) | LGPL compliance requirement; Version shown in the status bar only; no licence notices | missing |
| F0.11 | Per-user NSIS installer, no admin rights | Hard constraint; `tauri.conf.json` `installMode: currentUser`; CI installs silently and smoke-tests | done |
| F0.12 | Automatic updates (check, download, install, restart) | Hard constraint; No updater plugin or config; no Authenticode signing by design (private use, ADR-0001 amendment) | missing |
| F0.13 | GPU / hardware-acceleration capability detection with graceful fallback | Adapter/backend/software (WARP) detected and shown; preview `unavailable` fallback; no hardware decode/encode yet | partial |
| F0.14 | Single-instance handling (opening a project file focuses the running app) | | missing |
| F0.15 | File association for the project file extension | devops; Extension not decided (ADR-0002, H5) | missing |

### Projects

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.16 | Home screen: create project, recent projects (cover thumbnail, name, duration, last modified) | App opens straight into the editor | missing |
| F0.17 | Create a new project with default settings | `new_project` command (history cleared, clean); File > New / Ctrl+N with the unsaved-changes prompt (`ui/src/features/project/fileActions.ts`, Vitest). Manual check in `pnpm dev` on Windows done 2026-11-22 | done |
| F0.18 | Project settings: canvas resolution, aspect ratio, frame rate | Changing them later must not break the timeline; `set_project_settings` is one undo entry and keeps clip flicks bit-exact (app-core proptest); File > Project settings… dialog: presets 16:9 / 4K / 9:16 Shorts / 1:1 or custom even 16-8192, rational frame rates incl. 23.976 and 29.97, derived aspect ratio (`ProjectSettingsDialog.tsx`, `settings.ts`, Vitest). Manual check in `pnpm dev` on Windows done 2026-11-22 | done |
| F0.19 | Save project (explicit save + Save As) | ADR-0002: atomic save (temp + rename, `.v<N>.bak`); `save_project` / `save_project_as`; File > Save / Save As…, Ctrl+S / Ctrl+Shift+S, native save dialog filtered to `.cutframe` (default name = project name), Save without a file runs Save As; dirty `*` in the window title. Manual save → quit → open check in `pnpm dev` on Windows done 2026-11-22 | done |
| F0.20 | Open project from file and from the recent list | `open_project` (load → migrate → validate; failed open changes nothing, errors translated by `IpcError.kind`); File > Open… / Ctrl+O with native dialog and unsaved prompt. Missing: the recent-projects list (task 0120). Manual check in `pnpm dev` on Windows done 2026-11-22 | partial |
| F0.21 | Autosave at a configurable interval | | missing |
| F0.22 | Crash recovery: offer to restore the last autosave | | missing |
| F0.23 | Rename / duplicate / delete project from the home screen | Rename only (`RenameProject` from the inspector); no home screen | partial |
| F0.24 | Project-file schema version and forward migrations | ADR; `schemaVersion` + `migrate()` chain, v1 fixture test, rejects newer files; ADR-0002 Proposed | done |
| F0.25 | Detect missing media on open; relink dialog (single file and whole folder) | No media in the model yet | missing |
| F0.26 | Unsaved-changes prompt on close | Dirty = history position differs from the saved one; Rust stops the close and emits `quit-requested`, the UI asks Save / Don't save / Cancel (Save chains into Save As; a cancelled or failed save keeps the window open), then `confirm_close`; the same prompt guards New and Open (`fileActions.ts`, `UnsavedChangesDialog.tsx`, Vitest). Manual check in `pnpm dev` on Windows done 2026-11-22 | done |

### Edit history

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.27 | Undo / redo for every model change (Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y) | Architecture rule 1; All 4 commands undoable (`Document`), history cap 500; Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y and top-bar buttons | done |
| F0.28 | Continuous gestures (drag, slider) coalesce into one undo step | `Batch` + gestures (`EditPhase`, `cancel_gesture`) in `app-core`, UI gesture helper (task 0006); new drags must use it | done |
| F0.29 | Multi-step operations are one atomic transaction | `Batch` command in `app-core` (task 0006) | done |
| F0.30 | Undo/redo menu items show the action name | Undo/redo tooltips show `undoLabel`/`redoLabel` | done |
| F0.31 | History panel (list of steps, jump back) | | missing |

### Preferences and storage

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.32 | Preferences dialog (general, editing defaults, performance, cache, shortcuts) | | missing |
| F0.33 | Default image duration, default transition duration | | missing |
| F0.34 | Cache location and size; "clear cache" | Thumbnails, waveforms, proxies; No cache exists yet | missing |
| F0.35 | UI language setting (English first, Polish later) | Strings centralised in `i18n/en.ts`, so a locale switch is easy to add | missing |

---

## Tier 1 — Core editing (MVP)

### Media import

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.01 | Import video, audio and images via file dialog (multi-select) | | missing |
| F1.02 | Drag and drop files from the OS into the media panel | | missing |
| F1.03 | Drag and drop files from the OS directly onto the timeline | | missing |
| F1.04 | Video containers: MP4, MOV, MKV, WebM, AVI, M4V | | missing |
| F1.05 | Video codecs (decode): H.264, HEVC, VP8/VP9, AV1, ProRes, MPEG-2/4, MJPEG | Decoders are LGPL-safe | missing |
| F1.06 | Audio formats: MP3, WAV, AAC/M4A, FLAC, OGG/Opus | | missing |
| F1.07 | Image formats: JPG, PNG, WebP, BMP, TIFF, GIF (animated) | | missing |
| F1.08 | HEIC/HEIF photos (phone pictures) | Check FFmpeg 7.x support | missing |
| F1.09 | Media probing: duration, resolution, frame rate, codec, audio channels/sample rate | | missing |
| F1.10 | Rotation metadata honoured (portrait phone videos) | | missing |
| F1.11 | Variable frame rate sources play and cut correctly | | missing |
| F1.12 | Non-square pixel aspect ratio and anamorphic sources displayed correctly | | missing |
| F1.13 | Corrupt / unsupported files rejected with a clear message | | missing |

### Media panel

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.14 | Grid of media with thumbnail, name, duration, type icon | `media://` | missing |
| F1.15 | List view alternative | | missing |
| F1.16 | Hover-scrub thumbnail preview (skimming) | | missing |
| F1.17 | Preview a media item in the player before adding it | | missing |
| F1.18 | "Used in timeline" badge | | missing |
| F1.19 | Sort and filter (type, name, date, duration) | | missing |
| F1.20 | Search by name | | missing |
| F1.21 | Remove media from project (with warning if used) | | missing |
| F1.22 | Show in file explorer | | missing |
| F1.23 | Background thumbnail and waveform generation with progress | | missing |

### Timeline

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.24 | Timeline with a main video track | Video/audio/text track kinds exist; no "main track" concept | partial |
| F1.25 | Add media to the timeline (drag, "+" button, append at end, insert at playhead) | | missing |
| F1.26 | Overlay video tracks above the main track (picture-in-picture) | Multiple video tracks can be added by hand; no compositing yet | partial |
| F1.27 | Audio tracks below the main track | Audio track kind in the model; no audio | partial |
| F1.28 | Tracks created automatically when dropping onto empty space; empty tracks removed | Manual add/remove track buttons only | partial |
| F1.29 | Time ruler with adaptive timecode labels | Flicks internally; `renderer.ts` `drawRuler` with adaptive step | done |
| F1.30 | Playhead; click/drag on ruler and tracks to seek | Click/drag on the timeline canvas seeks; playhead is UI-only (`playheadStore`) | done |
| F1.31 | Auto-scroll timeline to follow the playhead during playback | | missing |
| F1.32 | Zoom (Ctrl+wheel, slider, zoom-to-fit) | Ctrl+wheel zoom around the cursor; no slider or zoom-to-fit | partial |
| F1.33 | Horizontal and vertical scroll | Shift/horizontal wheel scrolls time, vertical wheel scrolls tracks | done |
| F1.34 | Filmstrip thumbnails on video clips | `media://` | missing |
| F1.35 | Waveforms on audio clips and on video clips with audio | `media://` | missing |
| F1.36 | Clip label (name, duration, speed badge, effect badges) | Clips are drawn with their name (`drawClips`); no badges; clips cannot be added yet | partial |
| F1.37 | Select a clip; multi-select with Ctrl/Shift; marquee (rubber-band) selection | | missing |
| F1.38 | Select all (Ctrl+A); deselect (Esc) | | missing |
| F1.39 | Move clips by drag within and between tracks | | missing |
| F1.40 | Trim start/end by dragging clip edges, limited by source length | | missing |
| F1.41 | Split at playhead (Ctrl+B) — selected clip or all tracks | | missing |
| F1.42 | Delete clip (leaves gap on overlay tracks) | | missing |
| F1.43 | Ripple delete (closes the gap) | | missing |
| F1.44 | Magnetic main track (no gaps; clips reorder by drag) with on/off toggle | | missing |
| F1.45 | Snapping to playhead, clip edges, markers, with on/off toggle | | missing |
| F1.46 | Copy / cut / paste / duplicate clips | | missing |
| F1.47 | Trim to playhead: delete left / delete right of playhead (Q / W) | | missing |
| F1.48 | Select and delete a gap | | missing |
| F1.49 | Linked video + audio of the same source move/trim together; link on/off toggle | | missing |
| F1.50 | Detach audio from a video clip onto its own track | | missing |
| F1.51 | Replace clip media while keeping position, duration and effects | | missing |
| F1.52 | Track controls: mute, hide, lock | `muted`/`hidden` fields in the model; no commands, UI or lock | partial |
| F1.53 | Reorder tracks | | missing |
| F1.54 | Frame-step (Left/Right), jump to previous/next edit (Up/Down), go to start/end (Home/End) | | missing |
| F1.55 | Hover preview: skimming the timeline shows that frame in the preview, with on/off toggle | | missing |
| F1.56 | Right-click context menu on clips and tracks | | missing |
| F1.57 | Timeline length = end of last clip; project duration shown | `Project::duration()`, shown in the inspector | done |

### Preview

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.58 | Real-time playback of the composed timeline with audio/video sync | Architecture rules 2–4; Native wgpu preview window renders a test pattern only | missing |
| F1.59 | Play/pause (Space), current/total timecode display | Playhead timecode shown; Play button disabled | partial |
| F1.60 | Dropped-frame handling: audio is the master clock, video skips | | missing |
| F1.61 | Preview quality setting (full / half / quarter resolution) | | missing |
| F1.62 | Preview fit/zoom levels | Preview letterboxes to the canvas aspect; no zoom levels | partial |
| F1.63 | Full-screen preview | | missing |
| F1.64 | Direct manipulation in the preview: drag to move, corner handles to scale, handle to rotate | | missing |
| F1.65 | Selecting an element in the preview selects its clip on the timeline | | missing |
| F1.66 | Snapping guides in the preview (centre, edges) | | missing |
| F1.67 | Loop playback | | missing |
| F1.68 | J/K/L shuttle (reverse / pause / forward, repeated presses speed up) | | missing |
| F1.69 | Audio scrubbing while dragging the playhead | | missing |
| F3.45 | Hardware-accelerated decoding (D3D11VA / D3D12VA on Windows) for HEVC, H.264, AV1 with software fallback | Moved from Tier 3 on 2026-11-22 (4K HEVC phone footage); ID kept. Zero-copy path decided in 0310; `docs/design/media-import.md` §5 | missing |
| F3.31 | HDR sources (PQ/HLG, Rec.2020, Dolby Vision 8.x base layer) with tone mapping to SDR | Moved from Tier 3 on 2026-11-22 (phone records HDR); ID kept. One simple operator in the shared render graph so preview and export match; probe data in `docs/design/media-import.md` §3. Refinement and 10-bit pipeline: F3.30 | missing |

### Basic clip properties (inspector)

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.70 | Transform: position, scale, rotation | | missing |
| F1.71 | Non-uniform scale (lock/unlock aspect) | | missing |
| F1.72 | Opacity | | missing |
| F1.73 | Flip horizontal / vertical; rotate 90° | | missing |
| F1.74 | Crop (free and fixed ratios) with crop handles in the preview | | missing |
| F1.75 | Fit / fill / stretch to canvas | | missing |
| F1.76 | Canvas background: colour, blur of the clip itself, image | | missing |
| F1.77 | Clip volume (percent / dB) | | missing |
| F1.78 | Audio fade in / fade out | | missing |
| F1.79 | Constant speed change (e.g. 0.1×–100×) with duration updated | | missing |
| F1.80 | Preserve pitch when changing speed (toggle) | Time-stretch on the audio path | missing |
| F1.81 | Reverse clip | Needs reverse decode / cached render | missing |
| F1.82 | Freeze frame (insert a still of the current frame) | | missing |
| F1.83 | Default duration for still images; image duration change | | missing |
| F1.84 | Reset a property / reset all | | missing |
| F1.85 | Numeric entry for every slider (type exact values) | | missing |

### Export

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.86 | Export dialog: file name, destination folder | Export button disabled | missing |
| F1.87 | Resolution: 480p, 720p, 1080p, 2K, 4K (keeping project aspect) | | missing |
| F1.88 | Frame rate: 24, 25, 30, 50, 60 fps | | missing |
| F1.89 | Bitrate: low / recommended / high / custom | | missing |
| F1.90 | H.264 export in MP4 | LGPL: hardware/OS encoder (Media Foundation's software H.264 encoder ships with Windows), never libx264; OpenH264 excluded (F28) | missing |
| F1.91 | HEVC export in MP4/MOV | LGPL: hardware/OS encoder only | missing |
| F1.92 | AAC audio in the export | | missing |
| F1.93 | Export uses the same render graph as preview | Architecture rule 3 | missing |
| F1.94 | Progress with ETA, cancel, background export while editing | Long-running job | missing |
| F1.95 | Hardware-accelerated encoding with software fallback | | missing |
| F1.96 | Export audio only (WAV, MP3, AAC) | | missing |
| F1.97 | Export the current frame as an image (PNG/JPG) | | missing |
| F1.98 | Open the folder / play the file when the export finishes | | missing |
| F1.99 | Clear error when export fails (disk full, encoder missing) | | missing |

---

## Tier 2 — Essential creative tools

### Text

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.01 | Add a default text clip on a text track | | ? |
| F2.02 | Edit text in the inspector and inline in the preview | | ? |
| F2.03 | Multi-line text, text box with auto-wrap | | ? |
| F2.04 | Font family from system fonts and bundled fonts | Bundled fonts must have permissive licences | ? |
| F2.05 | Import custom font files | | ? |
| F2.06 | Size, colour, bold, italic, underline, case | | ? |
| F2.07 | Alignment, letter spacing, line spacing | | ? |
| F2.08 | Stroke / outline (colour, width) | | ? |
| F2.09 | Shadow (colour, opacity, blur, distance, angle) | | ? |
| F2.10 | Background box (colour, opacity, padding, corner radius) | | ? |
| F2.11 | Glow | | ? |
| F2.12 | Gradient fill | | ? |
| F2.13 | Emoji and full Unicode (RTL, CJK, Polish diacritics) | Text shaping | ? |
| F2.14 | Per-character / per-word styling (rich text) | | ? |
| F2.15 | Curved / arc text | | ? |
| F2.16 | Text style presets (one-click styles) | Effects-as-data | ? |
| F2.17 | Save own text style preset | | ? |
| F2.18 | Text animations: in, out, loop, with duration | | ? |
| F2.19 | Text templates (styled, animated title compositions) | Content | ? |
| F2.20 | Typewriter / per-character reveal animations | | ? |

### Transitions

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.21 | Add a transition on the cut between two adjacent clips (drag from library or click on cut) | | ? |
| F2.22 | Basic set: cross dissolve, fade to black/white, wipe, slide, push, zoom, blur, spin | `effect.json` + `shader.wgsl` | ? |
| F2.23 | Stylised set: glitch, flash, light leak, page curl, ink, shape reveals | | ? |
| F2.24 | Adjustable duration (handles/overlap uses media beyond the cut) | Model decision | ? |
| F2.25 | Apply transition to all cuts | | ? |
| F2.26 | Animated preview thumbnails in the transitions library | | ? |
| F2.27 | Transition parameters (direction, softness, colour) | | ? |

### Filters and adjustments

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.28 | Filter library (look presets) with intensity slider | | ? |
| F2.29 | Basic adjustments: brightness, contrast, saturation, exposure, temperature, tint | | ? |
| F2.30 | More adjustments: highlights, shadows, whites, blacks, vibrance, fade | | ? |
| F2.31 | Detail: sharpen, clarity, vignette, grain | | ? |
| F2.32 | Auto adjust (one-click auto exposure/colour) | | ? |
| F2.33 | Apply filter/adjustment to all clips | | ? |
| F2.34 | Filter or adjustment as its own segment on a track (adjustment layer affecting everything below for its duration) | Render-graph decision | ? |
| F2.35 | Save own adjustment preset | | ? |
| F2.36 | Import LUT (`.cube`) | | ? |
| F2.37 | Before/after comparison in the preview | | ? |

### Video effects

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.38 | Effects library: blur, shake, zoom pulse, glitch, VHS, RGB split, mirror, kaleidoscope, film grain, light leaks, sparkle, retro, etc. | Effects-as-data | ? |
| F2.39 | Effect parameters in the inspector | | ? |
| F2.40 | Effect applied to a clip vs. effect segment on its own track | | ? |
| F2.41 | Stack multiple effects; reorder; enable/disable | | ? |
| F2.42 | Mosaic / pixelate / blur region (privacy blur) | | ? |
| F2.43 | Frames and borders | | ? |

### Library UX (shared by text, transitions, filters, effects, stickers)

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.44 | Categories and animated previews | | ? |
| F2.45 | Search | | ? |
| F2.46 | Favourites | | ? |
| F2.47 | Drag from library to timeline / clip | | ? |

### Clip animations

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.48 | Clip in animations (fade, zoom, slide, spin, bounce, blur in) | | ? |
| F2.49 | Clip out animations | | ? |
| F2.50 | Combo / loop animations | | ? |
| F2.51 | Animation duration | | ? |
| F2.52 | Ken Burns pan-and-zoom for images | | ? |

### Keyframes

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.53 | Keyframes on transform and opacity | Model: animated properties | ? |
| F2.54 | Keyframes on volume | | ? |
| F2.55 | Keyframes on filter intensity, adjustments and effect parameters | | ? |
| F2.56 | Keyframes on mask and crop | | ? |
| F2.57 | Add/remove keyframe at playhead; previous/next keyframe navigation | | ? |
| F2.58 | Keyframe markers on the clip in the timeline; drag to retime | | ? |
| F2.59 | Easing: linear, ease in, ease out, ease in-out, hold | | ? |
| F2.60 | Custom Bézier easing / graph editor | | ? |
| F2.61 | Motion path shown in the preview | | ? |

### Compositing

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.62 | Picture-in-picture overlays on overlay tracks | | ? |
| F2.63 | Blend modes: normal, darken, multiply, colour burn, lighten, screen, colour dodge, add, overlay, soft light, hard light, difference, exclusion | | ? |
| F2.64 | Masks: linear, mirror (band), circle, rectangle with rounded corners, heart, star | | ? |
| F2.65 | Mask position, size, rotation, feather, invert | | ? |
| F2.66 | Chroma key (colour picker, intensity, edge/shadow, spill) | | ? |
| F2.67 | Images with transparency (PNG, WebP) composite correctly | | ? |
| F2.68 | Video with alpha channel (ProRes 4444, VP9 alpha WebM) | | ? |
| F2.69 | Split-screen layout presets | | ? |
| F2.70 | Canvas aspect ratio presets: 16:9, 9:16, 1:1, 4:3, 3:4, 4:5, 21:9, 2.35:1, custom | | ? |
| F2.71 | Preview guides: safe areas, rule-of-thirds grid, centre lines, social-app UI overlay | | ? |

### Stickers and overlays

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.72 | Sticker library (static and animated) | Out of scope (private hobby use): licensed content is a public-product need; use own files. Was: Content | n/a |
| F2.73 | Import own stickers (PNG, GIF, APNG, animated WebP) | | ? |
| F2.74 | Sticker in/out/loop animations | | ? |
| F2.75 | Shapes (rectangle, circle, arrow, line) with fill/stroke | | ? |

### Audio basics

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.76 | Background music on audio tracks | | ? |
| F2.77 | Extract audio from a video file into the media panel | | ? |
| F2.78 | Voiceover recording from a microphone (device picker, level meter, countdown) | cpal input | ? |
| F2.79 | Fade handles directly on timeline clips | | ? |
| F2.80 | Volume line on the clip (rubber band) with keyframes | | ? |
| F2.81 | Track volume and master volume | | ? |
| F2.82 | Audio level meters (master, per track) | | ? |
| F2.83 | Pan / balance; mono/stereo handling | | ? |
| F2.84 | Audio speed change with pitch options | | ? |
| F2.85 | Manual beat markers on music clips | | ? |
| F2.86 | Built-in sound effects library | Out of scope (private hobby use): licensed content is a public-product need; use own files. Was: Content | n/a |
| F2.87 | Built-in royalty-free music library | Out of scope (private hobby use): licensed content is a public-product need; use own files or the YouTube Audio Library. Was: Content | n/a |

### Timeline productivity

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.88 | Timeline markers (add, colour, label, jump, list) | | ? |
| F2.89 | Clip markers | | ? |
| F2.90 | Compound clip: combine selected clips into one; open/edit; uncombine | Model + render graph | ? |
| F2.91 | Group clips (move together without nesting) | | ? |
| F2.92 | Rename clips; clip colour labels | | ? |
| F2.93 | Track height and thumbnail size | | ? |
| F2.94 | Set clip duration numerically | | ? |
| F2.95 | Speed curves: presets (montage, hero, bullet, jump cut, flash in/out) and custom curve | | ? |
| F2.96 | Copy / paste attributes (effects, adjustments, transform) to other clips | | ? |
| F2.97 | Range (in/out) selection on the timeline | | ? |
| F2.98 | Select all clips forward / backward from the playhead | | ? |
| F2.99 | Ripple trim | | ? |

### Export (extended)

| ID | Feature | Notes | Status |
|---|---|---|---|
| F2.100 | Export a range (in/out) only | | ? |
| F2.101 | Save and reuse export presets | | ? |
| F2.102 | Estimated file size | | ? |
| F2.103 | Animated GIF export | Out of scope (private hobby use): GIF is not a YouTube format | n/a |
| F2.104 | Social presets (vertical 1080×1920, square, etc.) | | ? |
| F2.105 | Video cover / thumbnail: pick a frame or image, add text, embed in file and use in recent projects | | ? |

---

## Tier 3 — Advanced / pro

### Captions and speech

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.01 | Auto captions: local speech-to-text with language selection | Model (e.g. whisper.cpp, MIT); ADR | ? |
| F3.02 | Caption editor: list of lines, edit text, timing, split/merge lines | | ? |
| F3.03 | Batch styling of all captions; caption style presets | | ? |
| F3.04 | Word-level highlight (karaoke-style) captions | Needs word timestamps | ? |
| F3.05 | Import subtitles (SRT, ASS, LRC) | | ? |
| F3.06 | Export subtitles (SRT, TXT) and burn-in on export | | ? |
| F3.07 | Bilingual captions / translation | Out of scope (private hobby use): niche, needs a model or online service. Was: Model or online service | n/a |
| F3.08 | Lyrics recognition on music | Out of scope (private hobby use): niche, needs a model. Was: Model | n/a |
| F3.09 | Text-to-speech from a text clip | Model / OS voices | ? |
| F3.10 | Transcript-based editing: delete words in the transcript to cut the video | | ? |
| F3.11 | Remove filler words | | ? |
| F3.12 | Auto remove silences / pauses | No model needed (energy detection) | ? |

### Pro audio

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.13 | Loudness normalisation to a target (LUFS) | | ? |
| F3.14 | Noise reduction | DSP or model | ? |
| F3.15 | Voice enhancement | | ? |
| F3.16 | Automatic ducking (music lowers under voice) | | ? |
| F3.17 | Voice changer / voice effects (pitch, robot, echo, reverb, telephone, chipmunk, monster) | | ? |
| F3.18 | Equaliser | | ? |
| F3.19 | Compressor / limiter | | ? |
| F3.20 | Automatic beat detection on music | | ? |
| F3.21 | Auto-cut to the beat (place cuts on beat markers) | | ? |
| F3.22 | Audio fade curve shapes | | ? |
| F3.23 | Vocal / instrumental separation | Out of scope (private hobby use): heavy ML model for little gain. Was: Model | n/a |
| F3.24 | Audio mixer panel | | ? |

### Pro colour

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.25 | HSL: per-hue-range hue/saturation/lightness | | ? |
| F3.26 | Curves: master, R, G, B, hue-vs-sat, etc. | | ? |
| F3.27 | Colour wheels: lift, gamma, gain, offset | Out of scope (private hobby use): pro grading | n/a |
| F3.28 | Scopes: waveform, vectorscope, histogram, RGB parade | Out of scope (private hobby use): pro grading | n/a |
| F3.29 | Colour match between clips | Out of scope (private hobby use): pro grading | n/a |
| F3.30 | 10-bit (and higher) internal pipeline | Render graph | ? |
| F3.32 | HDR export | Out of scope (private hobby use): YouTube delivery stays SDR | n/a |
| F3.33 | Colour-space aware pipeline (Rec.709, sRGB, Display P3) | Out of scope (private hobby use): wide-gamut delivery not needed; Rec.709 only. Was: ADR | n/a |

### Motion and image processing

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.34 | Video stabilisation (strength levels) | Analysis job | ? |
| F3.35 | Smooth slow motion (optical-flow frame interpolation) | | ? |
| F3.36 | Frame blending for speed changes | | ? |
| F3.37 | Motion blur | | ? |
| F3.38 | Motion tracking: attach text/sticker/effect to a moving object | | ? |
| F3.39 | Tracked privacy blur (follow a face/plate) | | ? |
| F3.40 | Auto reframe to another aspect ratio following the subject | | ? |
| F3.41 | Scene detection: auto split a clip at scene cuts | | ? |
| F3.42 | Video denoise | | ? |
| F3.43 | Deflicker | Out of scope (private hobby use): pro footage repair | n/a |
| F3.44 | Lens distortion correction | Out of scope (private hobby use): pro footage repair | n/a |

### Performance

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.46 | Proxy media: generate, toggle, auto-use for 4K / heavy codecs | | ? |
| F3.47 | Render cache: pre-render heavy timeline sections | | ? |
| F3.48 | Background analysis jobs queue (waveforms, proxies, stabilisation, transcription) | ADR: long-running jobs | ? |
| F3.49 | Smooth editing of long timelines (1h+, hundreds of clips) | | ? |
| F3.50 | Memory and GPU budget management with graceful degradation | | ? |

### Pro editing workflow

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.51 | Insert vs overwrite edits; three-point editing | Out of scope (private hobby use): pro editing workflow | n/a |
| F3.52 | Roll, slip and slide edits | Out of scope (private hobby use): pro editing workflow | n/a |
| F3.53 | Timecode entry for move/seek ("+1:00" style) | Out of scope (private hobby use): pro editing workflow | n/a |
| F3.54 | Customisable keyboard shortcuts with presets resembling other popular editors | Out of scope (private hobby use): pro editing workflow; default shortcuts are enough | n/a |
| F3.55 | Media folders/bins, tags, ratings | Out of scope (private hobby use): one-person projects do not need tags or ratings | n/a |
| F3.56 | Multiple timelines/sequences in one project | Out of scope (private hobby use): one video per project | n/a |
| F3.57 | Multicam editing (sync by audio, switch angles) | Out of scope (private hobby use): rare in hobby recording; audio sync (F3.58) stays | n/a |
| F3.58 | Sync clips by audio waveform | | ? |
| F3.59 | Project versions / snapshots | | ? |
| F3.60 | Collect files: copy project + all media into a folder (archive) | | ? |
| F3.61 | Interchange: import/export OpenTimelineIO / FCPXML / EDL | Out of scope (private hobby use): no other NLE to exchange projects with | n/a |
| F3.62 | Screen recording built into the app | Out of scope (private hobby use): use OBS | n/a |
| F3.63 | Webcam recording built into the app | Out of scope (private hobby use): use OBS | n/a |
| F3.64 | Teleprompter for voiceover/webcam recording | Out of scope (private hobby use): use OBS or a dedicated teleprompter app | n/a |

### Pro export

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.65 | ProRes / DNxHR export (MOV) | Out of scope (private hobby use): pro delivery format; YouTube takes H.264/HEVC. Was: FFmpeg native encoders, LGPL-safe | n/a |
| F3.66 | AV1 export | Out of scope (private hobby use): pro delivery format; YouTube takes H.264/HEVC. Was: Hardware AV1 or SVT-AV1 (BSD) | n/a |
| F3.67 | VP9 / WebM export | Out of scope (private hobby use): pro delivery format; YouTube takes H.264/HEVC. Was: libvpx (BSD) | n/a |
| F3.68 | Export with transparent background (ProRes 4444, WebM alpha, PNG sequence) | Out of scope (private hobby use): pro delivery format | n/a |
| F3.69 | Image sequence export | Out of scope (private hobby use): pro delivery format | n/a |
| F3.70 | Export queue / batch export | Out of scope (private hobby use): one video at a time | n/a |
| F3.71 | Higher frame rates (120 fps) and up to 8K | Out of scope (private hobby use): beyond YouTube hobby needs | n/a |
| F3.72 | Audio stems export (per track) | Out of scope (private hobby use): pro delivery format | n/a |
| F3.73 | Chapter markers embedded in MP4 | Out of scope (private hobby use): YouTube chapters come from the description | n/a |

### Localisation and accessibility

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.74 | Polish UI locale | `CLAUDE.md`: later | ? |
| F3.75 | Keyboard-only operation of all panels | | ? |
| F3.76 | Screen-reader labels; high-contrast / UI scale options | Out of scope (private hobby use): single known user | n/a |

---

## Tier 4 — Smart / AI features

Every row here needs a model ADR (licence, size, download vs bundle, CPU/GPU cost, offline).

| ID | Feature | Notes | Status |
|---|---|---|---|
| F4.01 | Background removal for people (auto cutout) | Model | ? |
| F4.02 | Custom cutout: click/brush to select an object, track it through the clip | Model (segmentation) | ? |
| F4.03 | Object removal from video (inpainting) | Model | ? |
| F4.04 | Auto colour correction / auto enhance | Model or heuristics | ? |
| F4.05 | Video upscaling / quality enhancement | Model | ? |
| F4.06 | AI relighting | Model | ? |
| F4.07 | Face retouch (smooth skin, whiten teeth, reshape) and makeup | Model (face landmarks) | ? |
| F4.08 | Body reshape | Model | ? |
| F4.09 | Body effects that follow a person (outline, glow, trail) | Model (segmentation) | ? |
| F4.10 | Style transfer / cartoon / anime effects | Model | ? |
| F4.11 | Eye-contact correction | Model | ? |
| F4.12 | Auto highlights: turn a long video into short clips | Model | ? |
| F4.13 | Smart auto-edit: build a cut from selected media and music | Model | ? |
| F4.14 | Script-to-video (generate a video draft from text) | Model + content | ? |
| F4.15 | AI image / sticker generation | Model, likely online | ? |
| F4.16 | AI music generation | Model, likely online | ? |
| F4.17 | Voice cloning for TTS | Model; consent/legal review | ? |
| F4.18 | AI dubbing / translation with lip-sync | Model | ? |
| F4.19 | AI avatars / talking presenters | Model + content | ? |
| F4.20 | AI-written titles, descriptions, hashtags | LLM | ? |
| F4.21 | Smart cover/thumbnail suggestion | Model | ? |

---

## Tier 5 — Ecosystem and online

Mostly product, legal and infrastructure decisions; several may end up `n/a`.

| ID | Feature | Notes | Status |
|---|---|---|---|
| F5.01 | Template library: apply a template and swap in own media | Content + template format ADR | ? |
| F5.02 | Create and share own templates | | ? |
| F5.03 | Downloadable asset packs (effects, fonts, stickers, transitions) | Content + CDN | ? |
| F5.04 | Stock video / photo library | Content | ? |
| F5.05 | Brand kit (logos, colours, fonts) | | ? |
| F5.06 | User accounts / sign-in | | ? |
| F5.07 | Cloud storage and project sync across devices | | ? |
| F5.08 | Team spaces, shared projects, review comments on the timeline | | ? |
| F5.09 | Direct publishing to social/video platforms | | ? |
| F5.10 | Subscription / licensing for premium features | | ? |
| F5.11 | Mobile or web companion app | | ? |
| F5.12 | Third-party plugin / effect SDK | Effects-as-data makes this feasible | ? |
| F5.13 | Opt-in crash reporting and usage telemetry | Privacy review | ? |
