# Cutframe

A CapCut-style desktop video editor for Windows 10/11. Built with Rust, Tauri 2, React 19 and wgpu.

> **Cutframe** is a working codename. Decide the final name before the first public release:
> changing `identifier` in `src-tauri/tauri.conf.json` later breaks updates and app-data paths.

## Status

Stage 0 (skeleton): project model with undo/redo, typed IPC, an editor layout with a canvas
timeline, and a GPU-rendered test-pattern preview. Media import, playback and export come next.

## Development

Windows 10/11 x64 is the only supported development platform. Prerequisites:

- **MSVC Build Tools** (Visual Studio Build Tools, workload "Desktop development with C++")
- **Rust** via [rustup](https://rustup.rs); the pinned toolchain in `rust-toolchain.toml` (with
  rustfmt and clippy) is installed automatically on the first `cargo` call
- **Node.js 26** and **pnpm 12** (Node 26 no longer ships corepack: `npm install -g pnpm@12.21.0`, the version in `package.json`; make sure `%APPDATA%\npm` comes before `C:\Program Files\nodejs` in PATH so an old corepack shim does not shadow it)
- **WebView2 Runtime** (preinstalled on Windows 11 and current Windows 10)
- **LLVM 20** (`winget install -e --id LLVM.LLVM --version 20.1.8`): `libclang` for the FFmpeg
  bindings. Not 21 or newer: the bindgen 0.71 that `rusty_ffmpeg` 0.16 (FFmpeg 7.1) uses turns
  forward-declared structs such as `AVFormatContext` into opaque types with libclang 21+, and
  `rsmpeg` then fails to compile

```bash
pnpm install
pnpm setup:ffmpeg # pinned LGPL FFmpeg 7.1 into .ffmpeg/ (SHA-256 + licence checked), sets FFMPEG_* env
pnpm dev          # = tauri dev: Vite + the Rust app with hot reload
```

`pnpm setup:ffmpeg` (`scripts/fetch-ffmpeg.ps1`) is idempotent and needs no admin rights. It
downloads the build pinned in `scripts/ffmpeg.json`, refuses it on a hash mismatch or if any DLL is
not LGPL, and sets `FFMPEG_DIR`, `FFMPEG_INCLUDE_DIR`, `FFMPEG_LIBS_DIR`, `FFMPEG_LINK_MODE` (and
`LIBCLANG_PATH` if LLVM is in Program Files) as user variables, with `.ffmpeg\bin` on the user
`PATH`. Open a new terminal afterwards. The Rust build needs it: the installer bundles
`.ffmpeg/bin/*.dll` next to the `.exe`.

Useful commands:

```bash
pnpm check        # all local checks (cargo fmt, UI checks + build, clippy, tests)
pnpm bindings     # regenerate ui/src/bindings.ts after changing IPC commands or types
cargo test -p app-core
```

## Documentation

- `CLAUDE.md`: architecture rules, repository layout, agent responsibilities
- `docs/adr/`: architecture decisions (start with `0001-tech-stack.md`)
