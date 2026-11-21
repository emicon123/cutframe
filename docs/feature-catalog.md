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
    Foundation) or a permissively licensed software encoder (e.g. OpenH264).
  - *Model:* needs an ML model; licence, model size, CPU/GPU cost and offline use need an ADR.
  - *Content:* needs a library of licensed assets (music, stickers, stock footage, templates); that is
    a product/legal decision, not just code.
  - *ADR:* cross-cutting; starts with the `architect` agent.

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
| F0.01 | App starts into a main editor window with the standard layout: media/library panel, preview, inspector (properties), timeline | | ? |
| F0.02 | Resizable, collapsible panels; layout persisted between sessions | | ? |
| F0.03 | Window size/position/maximised state persisted | | ? |
| F0.04 | Dark theme (default), consistent design system | | ? |
| F0.05 | Main menu / command bar (File, Edit, View, Help) with shortcuts shown | | ? |
| F0.06 | Default keyboard shortcuts for all common actions | | ? |
| F0.07 | Keyboard shortcut reference panel | | ? |
| F0.08 | Non-blocking notifications (toasts) and readable error dialogs | | ? |
| F0.09 | Logging to a file in app data; "open logs folder" | | ? |
| F0.10 | About dialog with version and third-party licence notices (FFmpeg LGPL notice, source offer) | LGPL compliance requirement | ? |
| F0.11 | Per-user NSIS installer, no admin rights | Hard constraint | ? |
| F0.12 | Automatic updates (check, download, install, restart) | Hard constraint | ? |
| F0.13 | GPU / hardware-acceleration capability detection with graceful fallback | | ? |
| F0.14 | Single-instance handling (opening a project file focuses the running app) | | ? |
| F0.15 | File association for the project file extension | devops | ? |

### Projects

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.16 | Home screen: create project, recent projects (cover thumbnail, name, duration, last modified) | | ? |
| F0.17 | Create a new project with default settings | | ? |
| F0.18 | Project settings: canvas resolution, aspect ratio, frame rate | Changing them later must not break the timeline | ? |
| F0.19 | Save project (explicit save + Save As) | ADR: project-file format | ? |
| F0.20 | Open project from file and from the recent list | | ? |
| F0.21 | Autosave at a configurable interval | | ? |
| F0.22 | Crash recovery: offer to restore the last autosave | | ? |
| F0.23 | Rename / duplicate / delete project from the home screen | | ? |
| F0.24 | Project-file schema version and forward migrations | ADR | ? |
| F0.25 | Detect missing media on open; relink dialog (single file and whole folder) | | ? |
| F0.26 | Unsaved-changes prompt on close | | ? |

### Edit history

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.27 | Undo / redo for every model change (Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y) | Architecture rule 1 | ? |
| F0.28 | Continuous gestures (drag, slider) coalesce into one undo step | | ? |
| F0.29 | Multi-step operations are one atomic transaction | | ? |
| F0.30 | Undo/redo menu items show the action name | | ? |
| F0.31 | History panel (list of steps, jump back) | | ? |

### Preferences and storage

| ID | Feature | Notes | Status |
|---|---|---|---|
| F0.32 | Preferences dialog (general, editing defaults, performance, cache, shortcuts) | | ? |
| F0.33 | Default image duration, default transition duration | | ? |
| F0.34 | Cache location and size; "clear cache" | Thumbnails, waveforms, proxies | ? |
| F0.35 | UI language setting (English first, Polish later) | | ? |

---

## Tier 1 — Core editing (MVP)

### Media import

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.01 | Import video, audio and images via file dialog (multi-select) | | ? |
| F1.02 | Drag and drop files from the OS into the media panel | | ? |
| F1.03 | Drag and drop files from the OS directly onto the timeline | | ? |
| F1.04 | Video containers: MP4, MOV, MKV, WebM, AVI, M4V | | ? |
| F1.05 | Video codecs (decode): H.264, HEVC, VP8/VP9, AV1, ProRes, MPEG-2/4, MJPEG | Decoders are LGPL-safe | ? |
| F1.06 | Audio formats: MP3, WAV, AAC/M4A, FLAC, OGG/Opus | | ? |
| F1.07 | Image formats: JPG, PNG, WebP, BMP, TIFF, GIF (animated) | | ? |
| F1.08 | HEIC/HEIF photos (phone pictures) | Check FFmpeg 7.x support | ? |
| F1.09 | Media probing: duration, resolution, frame rate, codec, audio channels/sample rate | | ? |
| F1.10 | Rotation metadata honoured (portrait phone videos) | | ? |
| F1.11 | Variable frame rate sources play and cut correctly | | ? |
| F1.12 | Non-square pixel aspect ratio and anamorphic sources displayed correctly | | ? |
| F1.13 | Corrupt / unsupported files rejected with a clear message | | ? |

### Media panel

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.14 | Grid of media with thumbnail, name, duration, type icon | `media://` | ? |
| F1.15 | List view alternative | | ? |
| F1.16 | Hover-scrub thumbnail preview (skimming) | | ? |
| F1.17 | Preview a media item in the player before adding it | | ? |
| F1.18 | "Used in timeline" badge | | ? |
| F1.19 | Sort and filter (type, name, date, duration) | | ? |
| F1.20 | Search by name | | ? |
| F1.21 | Remove media from project (with warning if used) | | ? |
| F1.22 | Show in file explorer | | ? |
| F1.23 | Background thumbnail and waveform generation with progress | | ? |

### Timeline

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.24 | Timeline with a main video track | | ? |
| F1.25 | Add media to the timeline (drag, "+" button, append at end, insert at playhead) | | ? |
| F1.26 | Overlay video tracks above the main track (picture-in-picture) | | ? |
| F1.27 | Audio tracks below the main track | | ? |
| F1.28 | Tracks created automatically when dropping onto empty space; empty tracks removed | | ? |
| F1.29 | Time ruler with adaptive timecode labels | Flicks internally | ? |
| F1.30 | Playhead; click/drag on ruler and tracks to seek | | ? |
| F1.31 | Auto-scroll timeline to follow the playhead during playback | | ? |
| F1.32 | Zoom (Ctrl+wheel, slider, zoom-to-fit) | | ? |
| F1.33 | Horizontal and vertical scroll | | ? |
| F1.34 | Filmstrip thumbnails on video clips | `media://` | ? |
| F1.35 | Waveforms on audio clips and on video clips with audio | `media://` | ? |
| F1.36 | Clip label (name, duration, speed badge, effect badges) | | ? |
| F1.37 | Select a clip; multi-select with Ctrl/Shift; marquee (rubber-band) selection | | ? |
| F1.38 | Select all (Ctrl+A); deselect (Esc) | | ? |
| F1.39 | Move clips by drag within and between tracks | | ? |
| F1.40 | Trim start/end by dragging clip edges, limited by source length | | ? |
| F1.41 | Split at playhead (Ctrl+B) — selected clip or all tracks | | ? |
| F1.42 | Delete clip (leaves gap on overlay tracks) | | ? |
| F1.43 | Ripple delete (closes the gap) | | ? |
| F1.44 | Magnetic main track (no gaps; clips reorder by drag) with on/off toggle | | ? |
| F1.45 | Snapping to playhead, clip edges, markers, with on/off toggle | | ? |
| F1.46 | Copy / cut / paste / duplicate clips | | ? |
| F1.47 | Trim to playhead: delete left / delete right of playhead (Q / W) | | ? |
| F1.48 | Select and delete a gap | | ? |
| F1.49 | Linked video + audio of the same source move/trim together; link on/off toggle | | ? |
| F1.50 | Detach audio from a video clip onto its own track | | ? |
| F1.51 | Replace clip media while keeping position, duration and effects | | ? |
| F1.52 | Track controls: mute, hide, lock | | ? |
| F1.53 | Reorder tracks | | ? |
| F1.54 | Frame-step (Left/Right), jump to previous/next edit (Up/Down), go to start/end (Home/End) | | ? |
| F1.55 | Hover preview: skimming the timeline shows that frame in the preview, with on/off toggle | | ? |
| F1.56 | Right-click context menu on clips and tracks | | ? |
| F1.57 | Timeline length = end of last clip; project duration shown | | ? |

### Preview

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.58 | Real-time playback of the composed timeline with audio/video sync | Architecture rules 2–4 | ? |
| F1.59 | Play/pause (Space), current/total timecode display | | ? |
| F1.60 | Dropped-frame handling: audio is the master clock, video skips | | ? |
| F1.61 | Preview quality setting (full / half / quarter resolution) | | ? |
| F1.62 | Preview fit/zoom levels | | ? |
| F1.63 | Full-screen preview | | ? |
| F1.64 | Direct manipulation in the preview: drag to move, corner handles to scale, handle to rotate | | ? |
| F1.65 | Selecting an element in the preview selects its clip on the timeline | | ? |
| F1.66 | Snapping guides in the preview (centre, edges) | | ? |
| F1.67 | Loop playback | | ? |
| F1.68 | J/K/L shuttle (reverse / pause / forward, repeated presses speed up) | | ? |
| F1.69 | Audio scrubbing while dragging the playhead | | ? |

### Basic clip properties (inspector)

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.70 | Transform: position, scale, rotation | | ? |
| F1.71 | Non-uniform scale (lock/unlock aspect) | | ? |
| F1.72 | Opacity | | ? |
| F1.73 | Flip horizontal / vertical; rotate 90° | | ? |
| F1.74 | Crop (free and fixed ratios) with crop handles in the preview | | ? |
| F1.75 | Fit / fill / stretch to canvas | | ? |
| F1.76 | Canvas background: colour, blur of the clip itself, image | | ? |
| F1.77 | Clip volume (percent / dB) | | ? |
| F1.78 | Audio fade in / fade out | | ? |
| F1.79 | Constant speed change (e.g. 0.1×–100×) with duration updated | | ? |
| F1.80 | Preserve pitch when changing speed (toggle) | Time-stretch on the audio path | ? |
| F1.81 | Reverse clip | Needs reverse decode / cached render | ? |
| F1.82 | Freeze frame (insert a still of the current frame) | | ? |
| F1.83 | Default duration for still images; image duration change | | ? |
| F1.84 | Reset a property / reset all | | ? |
| F1.85 | Numeric entry for every slider (type exact values) | | ? |

### Export

| ID | Feature | Notes | Status |
|---|---|---|---|
| F1.86 | Export dialog: file name, destination folder | | ? |
| F1.87 | Resolution: 480p, 720p, 1080p, 2K, 4K (keeping project aspect) | | ? |
| F1.88 | Frame rate: 24, 25, 30, 50, 60 fps | | ? |
| F1.89 | Bitrate: low / recommended / high / custom | | ? |
| F1.90 | H.264 export in MP4 | LGPL: hardware/OS encoder or OpenH264, never libx264 | ? |
| F1.91 | HEVC export in MP4/MOV | LGPL: hardware/OS encoder only | ? |
| F1.92 | AAC audio in the export | | ? |
| F1.93 | Export uses the same render graph as preview | Architecture rule 3 | ? |
| F1.94 | Progress with ETA, cancel, background export while editing | Long-running job | ? |
| F1.95 | Hardware-accelerated encoding with software fallback | | ? |
| F1.96 | Export audio only (WAV, MP3, AAC) | | ? |
| F1.97 | Export the current frame as an image (PNG/JPG) | | ? |
| F1.98 | Open the folder / play the file when the export finishes | | ? |
| F1.99 | Clear error when export fails (disk full, encoder missing) | | ? |

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
| F2.72 | Sticker library (static and animated) | Content | ? |
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
| F2.86 | Built-in sound effects library | Content | ? |
| F2.87 | Built-in royalty-free music library | Content | ? |

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
| F2.103 | Animated GIF export | | ? |
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
| F3.07 | Bilingual captions / translation | Model or online service | ? |
| F3.08 | Lyrics recognition on music | Model | ? |
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
| F3.23 | Vocal / instrumental separation | Model | ? |
| F3.24 | Audio mixer panel | | ? |

### Pro colour

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.25 | HSL: per-hue-range hue/saturation/lightness | | ? |
| F3.26 | Curves: master, R, G, B, hue-vs-sat, etc. | | ? |
| F3.27 | Colour wheels: lift, gamma, gain, offset | | ? |
| F3.28 | Scopes: waveform, vectorscope, histogram, RGB parade | | ? |
| F3.29 | Colour match between clips | | ? |
| F3.30 | 10-bit (and higher) internal pipeline | Render graph | ? |
| F3.31 | HDR sources (PQ/HLG, Rec.2020) with tone mapping to SDR | ADR: colour management | ? |
| F3.32 | HDR export | | ? |
| F3.33 | Colour-space aware pipeline (Rec.709, sRGB, Display P3) | ADR | ? |

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
| F3.43 | Deflicker | | ? |
| F3.44 | Lens distortion correction | | ? |

### Performance

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.45 | Hardware-accelerated decoding (D3D11VA / DXVA2 on Windows) | | ? |
| F3.46 | Proxy media: generate, toggle, auto-use for 4K / heavy codecs | | ? |
| F3.47 | Render cache: pre-render heavy timeline sections | | ? |
| F3.48 | Background analysis jobs queue (waveforms, proxies, stabilisation, transcription) | ADR: long-running jobs | ? |
| F3.49 | Smooth editing of long timelines (1h+, hundreds of clips) | | ? |
| F3.50 | Memory and GPU budget management with graceful degradation | | ? |

### Pro editing workflow

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.51 | Insert vs overwrite edits; three-point editing | | ? |
| F3.52 | Roll, slip and slide edits | | ? |
| F3.53 | Timecode entry for move/seek ("+1:00" style) | | ? |
| F3.54 | Customisable keyboard shortcuts with presets resembling other popular editors | | ? |
| F3.55 | Media folders/bins, tags, ratings | | ? |
| F3.56 | Multiple timelines/sequences in one project | | ? |
| F3.57 | Multicam editing (sync by audio, switch angles) | | ? |
| F3.58 | Sync clips by audio waveform | | ? |
| F3.59 | Project versions / snapshots | | ? |
| F3.60 | Collect files: copy project + all media into a folder (archive) | | ? |
| F3.61 | Interchange: import/export OpenTimelineIO / FCPXML / EDL | | ? |
| F3.62 | Screen recording built into the app | | ? |
| F3.63 | Webcam recording built into the app | | ? |
| F3.64 | Teleprompter for voiceover/webcam recording | | ? |

### Pro export

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.65 | ProRes / DNxHR export (MOV) | FFmpeg native encoders, LGPL-safe | ? |
| F3.66 | AV1 export | Hardware AV1 or SVT-AV1 (BSD) | ? |
| F3.67 | VP9 / WebM export | libvpx (BSD) | ? |
| F3.68 | Export with transparent background (ProRes 4444, WebM alpha, PNG sequence) | | ? |
| F3.69 | Image sequence export | | ? |
| F3.70 | Export queue / batch export | | ? |
| F3.71 | Higher frame rates (120 fps) and up to 8K | | ? |
| F3.72 | Audio stems export (per track) | | ? |
| F3.73 | Chapter markers embedded in MP4 | | ? |

### Localisation and accessibility

| ID | Feature | Notes | Status |
|---|---|---|---|
| F3.74 | Polish UI locale | `CLAUDE.md`: later | ? |
| F3.75 | Keyboard-only operation of all panels | | ? |
| F3.76 | Screen-reader labels; high-contrast / UI scale options | | ? |

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
