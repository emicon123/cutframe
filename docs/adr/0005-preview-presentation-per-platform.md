# ADR-0005: Preview presentation per platform

- **Status:** Accepted (2026-11-22, after the spike results below)
- **Date:** 2026-11-21

## Context

ADR-0001 decides that the preview is rendered natively by wgpu in a child window over the UI's
preview rectangle. It names compositing with WebView2 as the main risk, asks for a Windows spike in
stage 0, and allows a simpler development-only mode on Linux. The stage-0 code implements three modes
(`PreviewMode` in `src-tauri/src/ipc/types.rs`):

- `native` (Windows): a `WS_CHILD | WS_CLIPSIBLINGS` Win32 window, a sibling of the WebView2
  controller above it in z-order (`src-tauri/src/preview/windows.rs`). wgpu (D3D12 only,
  `crates/engine/src/render/gpu.rs`) renders into it on the `preview-render` thread. The UI reports
  the rectangle with `set_preview_rect` and hides the window with `set_preview_visible`.
- `frames` (Linux development): the UI pulls `media://…/preview/frame?w=&h=`, which returns an
  8-byte header and raw RGBA8, at up to 30 fps and up to 1920×1080, and paints it on a 2D canvas
  (`ui/src/features/preview/PreviewArea.tsx`).
- `unavailable`: no GPU adapter. The app runs without a preview.

The frames mode and the visibility and input rules were not documented anywhere. The spike result is
recorded in "Spike results (2026-11-22)" below.

## Decision

1. **Windows ships only the `native` mode.** wgpu uses only the D3D12 backend on Windows. The software
   fallback is WARP, and `WGPU_BACKEND` is a debugging override only.
2. **Geometry:** the rectangle is in physical pixels relative to the webview's top-left corner,
   computed from `getBoundingClientRect() × devicePixelRatio`. The UI reports it on resize, on window
   resize and on DPI changes.
3. **Visibility** has two independent inputs: the UI's request (`set_preview_visible`) and a non-empty
   rectangle. The window is shown only when both allow it. `set_preview_rect` never shows a window
   that the UI has hidden (fixed in task 0003; it used to, see the review).
4. **Nothing in the DOM draws over the preview.** A menu, popover or dialog that has to overlap the
   preview area hides the preview first and shows it again after it closes. Selection handles, guides
   and safe areas are drawn by the engine.
5. **Linux `frames` mode is development-only.** It is never shipped, never tuned and never used for
   performance numbers. It stays functional because the Pi is a development machine. It uses the same
   renderer as the native path, so only presentation differs.
6. **Renderer independence:** renderers encode into any `wgpu::TextureView` (window surface,
   offscreen texture, encoder input). The presentation mode never changes what is rendered.
7. **Spike acceptance (ADR-0001 stage 0).** The native mode counts as validated when all of the
   following are checked on real Windows hardware, including one hybrid-GPU laptop, and the results
   are added to this ADR:
   - correct z-order over WebView2, with no flicker while panels or the window are resized
   - correct position after moving the window between monitors with different scale factors
   - minimize, restore, maximize and full-screen keep the window aligned
   - hiding for an overlapping dialog works, and the preview does not reappear on layout changes
   - 1080p at 60 Hz present rate without dropped frames on an integrated GPU, with the UI responsive
   - the CI `--smoke-test` presents frames into the child window on WARP, not only offscreen
   If the spike fails, Plan B from ADR-0001 (frames into the WebView through
   `ICoreWebView2SharedBuffer`) is decided in a new ADR.

**Open question:** pointer input over the preview (needed for on-canvas transform handles). Today the
child window returns `HTTRANSPARENT`. The design doc for on-canvas transforms decides between (a) the
child window's window procedure forwarding pointer events to the engine, which hit-tests them and
issues edits, and (b) the UI receiving them over a transparent region. It must work with rule 4.

## Spike results (2026-11-22)

**Setup:** release build `target/release/cutframe.exe` from `main` at commit `3df4a66`, driven by
PowerShell Win32 scripts with screenshots. Hybrid-GPU laptop: NVIDIA GeForce RTX 3070 Ti Laptop GPU
(driver 32.0.16.1742) and Intel Iris Xe Graphics (driver 31.0.101.3413), Windows 11 build 26300, one
internal display 2560×1440 at 125 % (120 DPI), 165 Hz. DXGI enumerates three adapters (dGPU, iGPU,
WARP). Default selection: RTX 3070 Ti, D3D12, `DiscreteGpu`.

**Diagnostics used** (commit `3df4a66`): `CUTFRAME_GPU=integrated|discrete` sets the GPU kind tried
first. `RUST_LOG=info,engine::preview=debug` logs `preview present rate` every 5 s: `avg_fps`,
`max_gap_ms`, `p99_gap_ms` and `long_gaps` (intervals longer than 1.5× the average).

| §7 check | Result | Notes |
|---|---|---|
| Z-order over WebView2, no flicker on resize | Pass | The `CutframePreview` child window matched the UI's preview rectangle to the pixel in every screenshot (initial, maximized, 1100×700, 1800×1100). After a splitter drag (20 steps at 16 ms) and a window resize sweep (12 steps at 16 ms), screenshots taken right after the last step showed the preview aligned with no gaps or stale areas. Still screenshots cannot prove the absence of flicker; a visual check during live drag-resize is recommended, not required. |
| Monitors with different scale factors | Not checked | Only one display attached. |
| Minimize, restore, maximize, full-screen | Pass, full-screen not checked | Restore from minimized returns to maximized with the preview aligned. The app has no full-screen mode yet. |
| Hiding for an overlapping dialog | Not checked | The UI has no dialog or menu that overlaps the preview yet. |
| Present rate on an integrated GPU, UI responsive | Pass, with caveat | See the table below. Every interval on the iGPU stayed below the 60 Hz frame budget (16.7 ms). Process CPU time about 2–3 s over about 20 s including start-up. The UI stayed interactive during the splitter drag; responsiveness was not measured numerically. Only the stage-0 test pattern is rendered, so decode and compositing load is not measured. |
| CI `--smoke-test` presents into the child window on WARP | Implemented | Commits `e1c7aa3`, `936f8bd`: CI fails unless the log shows `mode=Native`, `backend=Dx12` and at least 30 presented frames. The CI result on `main` was not checked in this session. |

Present rate, `AutoVsync`, 165 Hz display, three 5 s windows per run:

| Adapter | Surface | avg fps | max gap (ms) | p99 gap (ms) | long gaps |
|---|---|---|---|---|---|
| Iris Xe | 1432×730 (maximized) | 165.0 | 13.8 / 8.8 / 9.3 | 6.7–7.0 | 2 / 0 / 1 |
| Iris Xe | 1927×1196 (window larger than the screen) | 164.9–165.0 | 14.6 / 7.1 / 7.3 | 6.8–6.9 | 4 / 0 / 0 |
| RTX 3070 Ti | 1432×730 | 164.4–165.0 | 21.8 / 7.0 / 11.5 | 6.7–6.9 | 3 / 0 / 1 |

The 21.8 ms gap on the RTX 3070 Ti occurred only in the first window after start-up.

**Outcome:** no check failed, so the native mode is accepted and Plan B is not needed.

**Remaining checks.** Each must be done before the first public release, or when the feature it
depends on lands, whichever comes first. A failure of any of them reopens this ADR (Plan B per
ADR-0001).

- The CI `--smoke-test` on WARP is green on `main` (required by task 0005's Done-when).
- Correct position after moving the window between monitors with different scale factors.
- Full-screen keeps the preview aligned (when full-screen mode lands).
- Hiding for an overlapping dialog works and the preview does not reappear on layout changes (when the
  first menu, popover or dialog over the preview lands).
- The performance budget (1080p at 60 Hz without dropped frames, UI responsive) is re-measured on the
  integrated GPU with real decode and compositing (when video playback lands).
- Optional: a person watches a live drag-resize of panels and the window for flicker.

## Rationale

- Native presentation is the only option without a CPU round trip per frame (CLAUDE.md rule 2).
- D3D12-only avoids wgpu choosing a Vulkan adapter that cannot present to the window on hybrid-GPU
  laptops (fixed in commit `f5ee53a`) and matches ADR-0001.
- Pulling frames over `media://` is the smallest dev-mode that works on WebKitGTK and still keeps
  pixels out of JSON IPC.

Rejected:

- **`frames` mode on Windows:** a CPU read-back and copy per frame. It is reserved as the basis of
  Plan B only if the spike fails.
- **WebGPU inside the WebView:** the engine's textures and decode surfaces live in the native device,
  and moving them into the WebView would mean frames over a shared buffer (Plan B) anyway.
- **A separate top-level, borderless window:** focus, z-order and taskbar problems, and it moves
  independently of the main window.
- **A GTK child window on Linux:** effort for a development-only platform.

## Consequences and risks

- Any UI overlay that might cover the preview must use the hide/show protocol. This is a review item
  for `frontend`.
- The `frames` mode can hide performance problems. Performance budgets are measured on Windows only
  (ADR-0001).
- The native mode is validated on one hybrid-GPU laptop with the stage-0 test pattern only. The
  remaining checks in "Spike results" are release blockers, and any failure among them reopens this
  ADR.
