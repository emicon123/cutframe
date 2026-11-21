# ADR-0001: Tech stack — Rust + Tauri 2 + React 19 + wgpu

- **Status:** Accepted
- **Date:** 2026-11-21

## Context

We are building a CapCut-style desktop video editor. The target platform is **Windows 10/11 (x64)**.
Top-level requirements:

1. The most modern technology stack available.
2. **Easy installation:** one small installer, no admin rights, automatic updates.
3. Smooth real-time preview (1080p/4K, multiple tracks, GPU effects).
4. Development must also work on a Raspberry Pi 5 (Linux ARM64). Logic, UI and rendering must be
   cross-platform, even though the shipped product is Windows-only.

Options considered: C++20 + Qt 6/QML, Rust + Tauri 2 + React, Electron + native addon, C#/WinUI 3.

## Decision

**Rust (engine and app backend) + Tauri 2 (shell) + React 19/TypeScript (UI) + wgpu (GPU rendering) +
FFmpeg (media I/O).**

| Area | Technology |
|---|---|
| App shell | Tauri 2 (WebView2 on Windows, WebKitGTK on Linux) |
| UI | React 19, TypeScript, Vite, Tailwind CSS v4, shadcn/ui |
| UI state | Zustand (a mirror of the Rust-side model) |
| Timeline | Canvas 2D (not DOM, so it stays fast with thousands of clips) |
| Typed IPC | `tauri-specta` (TypeScript types generated from Rust) |
| Engine language | Rust (edition 2024, latest stable) |
| GPU rendering | `wgpu` (D3D12 on Windows, Vulkan/GLES on the Pi), WGSL shaders |
| Text / 2D graphics | `cosmic-text` + `glyphon`, `vello` (vectors, stickers, Lottie via `velato`) |
| Decode / encode | FFmpeg 7.x (LGPL, shared) via `rsmpeg`; HW: D3D11VA/D3D12VA, NVENC/QSV/AMF |
| Audio | `cpal` (WASAPI), resampling via FFmpeg `swresample` |
| AI | `whisper-rs` (auto-captions), `ort` (ONNX Runtime + DirectML: background removal) |
| Media index / cache | SQLite (`rusqlite`) |
| Project format | JSON (`serde`) with a `schemaVersion` field + migrations |
| Tests | `cargo test`, `insta` (snapshots), golden-frame tests; Vitest for the UI |
| Installer | Tauri bundler → **NSIS** `.exe` (`currentUser` mode, no admin), WebView2 `embedBootstrapper` |
| Updates | `tauri-plugin-updater` + GitHub Releases |
| Code signing | Azure Trusted Signing (removes the SmartScreen warning) |
| CI/CD | GitHub Actions: `windows-latest` + `tauri-action` (build, sign, release); Linux ARM64 checks |

## Architecture

```
┌──────────────── Tauri process (Rust) ──────────────────────────────┐
│                                                                    │
│  WebView2 (React UI)                    Preview window (wgpu)      │
│  media bin │ timeline │ inspector  ◄──► native child window placed │
│        ▲  invoke/events (tauri-specta)   over the UI's preview area│
│        │  media:// (thumbnails, waveforms)              ▲          │
│        ▼                                                │          │
│  app-core: project model, Command + Undo/Redo,          │ model    │
│            serialization, snapping, selection ──────────┘ snapshot │
│        │                                          (copy-on-write)  │
│  engine: media (FFmpeg, HW decode, cache, proxies)                 │
│          render graph (wgpu: transform, blend, filters,            │
│                        transitions, text, masks, keyframes)        │
│          audio (cpal, mixing, master clock)                        │
│          export (same graph → hardware encoder)                    │
│        │                                                           │
│  jobs: thumbnails, waveforms, proxies, AI (whisper, segmentation)  │
└────────────────────────────────────────────────────────────────────┘
```

Principles:

1. **Rust is the source of truth.** The UI sends commands (`invoke`). Rust applies them to the model,
   records them in the undo/redo history and emits events/patches back to the UI. The UI never mutates
   the model on its own.
2. **The preview is rendered natively by wgpu** in a child window placed over the preview rectangle of
   the UI. React reports the rectangle through `ResizeObserver` and a command. The engine draws the
   transform handles. Frames never go through IPC.
3. **Zero-copy on Windows:** hardware decode → GPU texture → wgpu, without a CPU round-trip.
4. **One render graph** for preview and export, so the export looks exactly like the preview.
5. **Audio is the master clock.** Video syncs to audio and drops frames when overloaded.
6. **Effects are data:** each filter/transition is a directory with `effect.json` (parameters, UI
   metadata) + `shader.wgsl`.
7. **Heavy UI assets** (thumbnails, waveforms) are served over a custom `media://` protocol, not
   through JSON IPC.

## Rationale

- **Modern:** Rust is memory-safe at C++ speed, wgpu is native WebGPU over D3D12/Vulkan, and
  React 19 + Tailwind v4 is the current state of the art for UI.
- **Easy installation:** Tauri produces a small NSIS installer (a few dozen MB including FFmpeg). It
  installs per user without admin rights and has a built-in auto-updater. WebView2 ships with
  Windows 10/11.
- **UI velocity:** React/TS + shadcn/ui makes it fast to build a rich CapCut-style interface.
- **Cross-platform:** the same code builds on the Pi (Linux ARM64) for development and on Windows for
  release.

Rejected:
- **Qt 6/C++:** mature, but an older ecosystem, slower UI development and no memory safety.
- **Electron:** installer of ~150 MB+ and high RAM use. It has the same preview problem as Tauri.
- **C#/WinUI 3:** Windows-only, so there would be no way to develop on the Pi.

## Consequences and risks

- **Main risk: compositing the native wgpu window with WebView2.** This must be validated first, as a
  spike in stage 0 on Windows. Plan B: pass frames to the WebView through `ICoreWebView2SharedBuffer`
  (shared memory) and display them with WebGPU in the UI. This is slower but has no compositing issues.
- On Linux (Pi) the child-window mechanism works differently (GTK). A simpler development-only preview
  mode is acceptable there.
- The Pi 5 has hardware decode for HEVC only, so H.264 is decoded in software. Measure performance
  on Windows only.
- **Licensing:** FFmpeg is used only as an LGPL build, dynamically linked (DLLs next to the `.exe`).
  No x264/x265 (GPL). Encoding goes through NVENC/QSV/AMF, with OpenH264 as the fallback.
- **Branding:** use our own name and assets. No CapCut logos, names or materials in the product.
- MSI (WiX) installers can only be built on Windows, so releases are built in CI on `windows-latest`.
