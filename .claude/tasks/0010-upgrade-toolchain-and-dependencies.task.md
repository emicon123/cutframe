# Task 0010: Upgrade the toolchain and all dependencies

**Tier:** chore · **Agents:** `devops` (lead), `backend`, `frontend`; `architect` only if an upgrade
forces an architecture change · **Depends on:** nothing
**Spec:** `docs/adr/0001-tech-stack.md`, `CLAUDE.md` (hard constraints, quality gates), `deny.toml`

## Why

Upgrades are cheapest while the codebase is small. The first release run (v0.1.0, 2026-11-22) also
warned that `pnpm/action-setup@v4` still targets the deprecated Node.js 20 runtime.

## Snapshot (2026-11-22, re-check before starting)

| Area | Current | Latest seen | Notes |
|---|---|---|---|
| Rust toolchain (`rust-toolchain.toml`, `rust-version`) | 1.100 | 1.100 | up to date; re-check |
| Rust crates (`Cargo.lock`) | — | `cargo update` has 1 compatible bump (hashbrown) | 4 transitive crates pinned behind latest (generic-array, toml 0.8, toml_datetime, toml_edit) by upstream |
| `tauri` / `tauri-build` / `@tauri-apps/api` / `@tauri-apps/cli` | 2.12 / 2.7 / 2.12.5 / 2.12.6 | 2.x latest; 3.0.0-alpha.6 exists | stay on Tauri 2; **do not** adopt 3.x alpha |
| `specta` / `tauri-specta` / `specta-typescript` | =2.0.0-rc.28 / =2.0.0-rc.29 / =0.0.15 | same | exact pins; bump together only, then `pnpm bindings` |
| `wgpu` | 30 | 30.0.2 | check for a newer major |
| `windows` | 0.62 | 0.62.2 | |
| pnpm (`packageManager`) | 11.24.0 | 12.21.0 | major bump: lockfile format, `pnpm-workspace.yaml` settings |
| Node (`engines`, CI `node-version`) | 24 | 24.22.3 | move to the current LTS line if it is newer than 24 |
| TypeScript | ~6.0.3 | 7.0.2 | major (native compiler); check typescript-eslint, Vite, Vitest support |
| Other UI deps (React 19, Vite 8, Vitest 5, Tailwind 4, ESLint 10, …) | — | `pnpm -r outdated` showed only TypeScript | re-check |
| `actions/checkout` | v5 | v7.0.1 | |
| `actions/setup-node` | v5 | v7.1.0 | |
| `pnpm/action-setup` | v4 | v6.1.1 | v5 = v4 on Node 24; v6 adds pnpm 11+ support (Windows install fixes) |
| `actions-rust-lang/setup-rust-toolchain` | v1 | v2.0.1 | |
| `actions/upload-artifact` | v4 | v7.0.2 | |
| `tauri-apps/tauri-action` | v0 | action-v1.0.0 | check input/env changes (`GITHUB_TOKEN`, `tagName`, `releaseDraft`) |
| `EmbarkStudios/cargo-deny-action` | v2 | v2.1.3 | |

Not yet dependencies (pick current versions when they land, not here): FFmpeg 7.x / `rsmpeg`, `cpal`.

## Scope

- **devops:** `rust-toolchain.toml` + workspace `rust-version`; root `package.json` (`packageManager`,
  `engines`, `@tauri-apps/cli`); every action in `.github/workflows/ci.yml` and `release.yml` (keep
  the two in step); `deny.toml` if new licences or duplicate versions appear.
- **backend:** `Cargo.toml` workspace deps + `cargo update`; fix breaking changes in `app-core`,
  `engine`, `src-tauri`; re-run `pnpm bindings` and commit `ui/src/bindings.ts`.
- **frontend:** `ui/package.json` (TypeScript 7 and the rest of `pnpm -r outdated`); fix type/lint
  breaks; keep Prettier output stable or reformat in a separate commit.
- One commit per area/major bump (Conventional Commits, e.g. `chore(deps): …`, `ci: …`) so a bad
  upgrade can be reverted alone.

## Constraints

- No pre-release or alpha versions (Tauri 3 alpha, RC-only crates other than the existing specta pins).
- LGPL-only / no GPL: `cargo deny check` must pass; new npm deps checked for licence too.
- Must still build and pass tests on Linux ARM64 (Raspberry Pi 5 / CI `ubuntu-24.04-arm`).
- Do not change the bundle identifier, product name or installer mode.

## Done when

- every row above is at its latest stable version, or the row says why it is held back
- `pnpm check` passes locally; Prettier check, UI build and `cargo deny check` pass
- CI is green on both jobs (Linux ARM64 and Windows installer + smoke test) with no Node.js
  deprecation annotations
- a `v*` tag test is **not** required; a local `pnpm tauri build` + silent install + `--smoke-test`
  (as in `ci.yml`) is enough to prove the release path
- `docs/adr/0001-tech-stack.md` version table/notes updated if it lists versions
