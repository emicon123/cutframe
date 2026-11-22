# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

For any task that leads to code changes:
- cross-cutting or architectural work (project model, IPC, render graph, audio, project-file format)
  starts with the `architect` agent — a design doc or ADR in `docs/`
- big tasks get one or more `<name>.task.md` files in `.claude/tasks/`
- then spawn `backend` / `frontend` / `devops` agents with the design as context

See "Agent orchestration" and "Task workflow" below.

## What this is

A CapCut-style desktop video editor, working codename **Cutframe**. Shipped product: **Windows 10/11
x64** with an easy NSIS installer (no admin rights) and automatic updates. Development happens on
Windows only (decision of 2026-11-22): Linux and the Raspberry Pi are no longer development or CI
platforms (ADR-0001 and ADR-0005 amendments). The legacy Linux code paths (`frames` preview mode)
are still in the code but unmaintained.

**Purpose: private, hobby use.** Cutframe is built for one person's own hobby video editing (e.g.
YouTube videos and Shorts), not as a commercial or publicly distributed product. Judge every feature
against that goal: keep what helps cut, polish and export a YouTube video quickly; drop professional
post-production workflows (interchange with other NLEs, broadcast/delivery formats, pro grading
suites), features that exist only for a public product (licensed content libraries, paid code
signing, accessibility/localisation for other audiences) and features other free tools already do
better (screen/webcam recording). Do not add such features back without the user asking.

**Canonical spec:** `docs/adr/` and `docs/design/` are the source of truth. If the prompt or the code
disagrees with them, say so before changing anything.

**Language:** chat with the user in Polish. All repository content (code, comments, docs, ADRs, agent
prompts, commit messages, UI strings) is in **English**. Polish can be added later as a UI locale.

## Hard constraints (these shape every decision)

- Windows 10/11 x64 is the only platform: shipping, development and CI. Do not add Linux CI jobs,
  and do not spend effort keeping Linux builds working.
- FFmpeg is LGPL only and dynamically linked. No GPL components (`deny.toml` enforces the crate
  licence policy).
- No CapCut names, logos or assets anywhere in the product.
- The installer must not need admin rights (NSIS `currentUser`) and the app must update itself.
- "Cutframe" (`app.cutframe.editor`) is a working codename; the final name must be chosen before the
  first public release, because changing the identifier later breaks updates and app-data paths.

## Stack (details and rationale: `docs/adr/0001-tech-stack.md`)

- **Engine + backend:** Rust (edition 2024), `wgpu` (WGSL shaders), FFmpeg 7.x via `rsmpeg`, `cpal`.
- **Shell:** Tauri 2. **UI:** React 19 + TypeScript + Vite + Tailwind v4 + shadcn/ui + Zustand, pnpm.
- **IPC:** typed via `tauri-specta`. Never hand-write TS types for Rust commands.
- **Packaging:** NSIS (`currentUser`), `tauri-plugin-updater`, release builds in GitHub Actions on `windows-latest`.

## Architecture rules (do not break without a new ADR)

1. The project model lives in Rust. The UI only sends commands and receives events. Every model change
   is a command with undo/redo.
2. Video frames never go through JSON IPC. wgpu renders the preview natively. Thumbnails and waveforms
   go through the `media://` protocol.
3. Preview and export use the same render graph.
4. The audio thread is real-time: no allocations, locks or I/O.
5. Effects and transitions are data: `effect.json` + `shader.wgsl`.
6. Time in the model and in IPC is `i64` flicks (1/705,600,000 s), never `f64` seconds.
7. FFmpeg is LGPL only and dynamically linked. No GPL components.
8. No CapCut names, logos or assets anywhere in the product.

## Agent orchestration

The main session is the orchestrator. Specialist agents live in `.claude/agents/`; subagents cannot
spawn further subagents (one level only). Each starts with fresh context, so pass this file, the task
and the architect's design/ADR.

| Work | Agent |
|---|---|
| ADRs, feature designs, project model / IPC / render-graph / audio impact, project-file migrations, test plans, Mermaid diagrams (`docs/adr/`, `docs/design/`) | **architect** |
| Rust: `app-core` model + undo/redo, `engine` (FFmpeg, wgpu, cpal, export), `src-tauri` (commands, events, preview window, `media://`), `effects/`, regenerated `bindings.ts` | **backend** |
| React 19 UI in `ui/` (canvas timeline, media bin, inspector, effects library, export dialog) | **frontend** |
| GitHub Actions, NSIS build + signing, updater, toolchain pinning, cargo-deny, setup scripts, `bundle`/updater config | **devops** |

**Sequencing gate:** for a cross-cutting feature the architect's design/ADR exists first. For any IPC
change the backend defines the command/event, regenerates `ui/src/bindings.ts` and tells the frontend
what changed before the frontend builds against it. Small changes inside one area go straight to that
area's agent.

## Quality gates (no agent skips these)

- **Local checks on the Windows machine are the primary gate**: they are faster than waiting for CI,
  so run them before every commit instead of pushing to find out.
- `pnpm check` must pass. It runs the CI checks in the same order: `cargo fmt --all --check`, `ui`
  typecheck + lint + Prettier check + Vitest + production build, then
  `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
- Before committing, also run locally what CI checks beyond `pnpm check`:
  `cargo deny --all-features check` (when dependencies changed; install with
  `cargo install cargo-deny@0.22.0 --locked`, the version CI pins) and, after any IPC change,
  `pnpm bindings` with the result committed.
- CI (`.github/workflows/ci.yml`, Windows only) is the second line of defence: a clean checkout, the
  same checks, the NSIS installer build and the installer smoke test. Read its result with `gh`.
- `ui/src/bindings.ts` is generated by tauri-specta — never hand-edit it and never hand-write TS
  types for Rust commands.
- Rust is the source of truth: model changes are commands with undo/redo; no model logic in
  TypeScript.
- Time in the model and in IPC is `i64` flicks, never `f64` seconds.
- Secrets never enter the repository; signing keys and tokens live in GitHub Actions secrets.
- Tests: `cargo test` (including `insta` snapshots and golden-frame tests), Vitest for the UI.
  `cargo test -p app-core` is the fast model loop with no GPU.

## Task workflow

Work items live in `.claude/tasks/` as `<name>.task.md` files; the first file alphabetically is the
next item. Big features get one or more task files there.

1. Read the first task file.
2. Spawn the appropriate agent(s) per the table above.
3. Verify with `pnpm check` (and the installer/CI path for packaging changes).
4. Move the task file to `.claude/tasks/done/` (or delete it if history is not useful).

Add tasks by dropping a `<name>.task.md` file into `.claude/tasks/`.

## Project structure

```
docs/adr/, docs/design/   decisions and feature designs           ── agent `architect`
Cargo.toml                Rust workspace                          ─┐
crates/app-core/          model, commands, undo/redo               │
crates/engine/            FFmpeg, wgpu, audio, export              ├─ agent `backend`
src-tauri/                Tauri shell, IPC, preview window         │
effects/                  effect.json + shader.wgsl               ─┘
ui/                       React app (Vite)                        ── agent `frontend`
ui/src/bindings.ts        GENERATED by tauri-specta: the backend ↔ frontend contract
.github/, scripts/, package.json, rust-toolchain.toml, deny.toml,
src-tauri/tauri.conf.json `bundle` + updater sections              ── agent `devops`
```

Full picture of modules and data flow: `docs/adr/0001-tech-stack.md`.

## Building and running

```bash
pnpm install
pnpm dev                       # tauri dev: Vite + Rust app with the native preview

pnpm check                     # cargo fmt, UI typecheck/lint/Prettier/test/build, clippy, cargo test
pnpm bindings                  # regenerate ui/src/bindings.ts after any IPC change
cargo test -p app-core         # fast model tests, no GPU needed
```

Windows installers are built in CI; a `v*` tag triggers `.github/workflows/release.yml` (draft GitHub
Release), and the tag must match the workspace version in `Cargo.toml`.

## Git

- Work happens on `main`; confirm no other agent is active before creating or switching branches.
- Use **Conventional Commits** (`feat:`, `fix:`, `chore:`, ...).
- **Committing and pushing to `main` is pre-approved**: no need to ask the user first. Commit
  finished work once the quality gates pass (`pnpm check`; for CI/packaging changes also the
  relevant installer/CI path), then `git push origin main`.
- Several sessions share this checkout: stage only the files your task changed (`git add <paths>`,
  never `git add -A` / `git add .`), and do not commit other sessions' uncommitted work.
- Before pushing, `git pull --rebase origin main`; if the rebase conflicts, stop and ask.
- Still ask first: force pushes, rewriting pushed history, deleting branches, and creating, moving
  or deleting `v*` tags (a tag starts a release).
