# ADR-0005: Preview presentation per platform

- **Status:** Proposed
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

The frames mode and the visibility and input rules are not documented anywhere, and the result of the
spike is not recorded.

## Decision

1. **Windows ships only the `native` mode.** wgpu uses only the D3D12 backend on Windows. The software
   fallback is WARP, and `WGPU_BACKEND` is a debugging override only.
2. **Geometry:** the rectangle is in physical pixels relative to the webview's top-left corner,
   computed from `getBoundingClientRect() × devicePixelRatio`. The UI reports it on resize, on window
   resize and on DPI changes.
3. **Visibility** has two independent inputs: the UI's request (`set_preview_visible`) and a non-empty
   rectangle. The window is shown only when both allow it. `set_preview_rect` never shows a window
   that the UI has hidden. (Today it does, see the review.)
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
- Until the spike checklist is recorded, the native mode is implemented but not validated.
