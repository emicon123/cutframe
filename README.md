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
- **Node.js 24** and **pnpm** (`corepack enable` picks the version from `package.json`)
- **WebView2 Runtime** (preinstalled on Windows 11 and current Windows 10)

```bash
pnpm install
pnpm dev          # = tauri dev: Vite + the Rust app with hot reload
```

Useful commands:

```bash
pnpm check        # all local checks (cargo fmt, UI checks + build, clippy, tests)
pnpm bindings     # regenerate ui/src/bindings.ts after changing IPC commands or types
cargo test -p app-core
```

## Documentation

- `CLAUDE.md`: architecture rules, repository layout, agent responsibilities
- `docs/adr/`: architecture decisions (start with `0001-tech-stack.md`)
