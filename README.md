# Cutframe

A CapCut-style desktop video editor for Windows 10/11. Built with Rust, Tauri 2, React 19 and wgpu.

> **Cutframe** is a working codename. Decide the final name before the first public release:
> changing `identifier` in `src-tauri/tauri.conf.json` later breaks updates and app-data paths.

## Status

Stage 0 (skeleton): project model with undo/redo, typed IPC, an editor layout with a canvas
timeline, and a GPU-rendered test-pattern preview. Media import, playback and export come next.

## Development

Linux (Debian 13 / Raspberry Pi OS):

```bash
./scripts/setup-dev-linux.sh   # system packages + Rust (needs sudo, run in a real terminal)
pnpm install
pnpm dev                       # = tauri dev: Vite + the Rust app with hot reload
```

On Linux the preview runs in a development mode (frames pulled over `media://`). The native GPU
preview window is Windows-only; Windows installers are built in CI (`.github/workflows/ci.yml`).

Useful commands:

```bash
pnpm check        # all UI and Rust checks (typecheck, lint, tests, fmt, clippy)
pnpm bindings     # regenerate ui/src/bindings.ts after changing IPC commands or types
cargo test -p app-core
```

## Documentation

- `CLAUDE.md`: architecture rules, repository layout, agent responsibilities
- `docs/adr/`: architecture decisions (start with `0001-tech-stack.md`)
