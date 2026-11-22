# Task: Lint Windows-only code in CI

**Agent:** `devops`.
**Source:** `docs/architecture-review.md` F34 (P0 #3).
**Touches:** `.github/workflows/ci.yml` only. Must not run in parallel with task 0005 (same file).

## Problem

`cargo clippy` and `cargo fmt --check` run only in the Linux job (`ci.yml:47-48`); the Windows job
(`ci.yml:57-71`) does not lint. `cfg(windows)` code (`src-tauri/src/preview/windows.rs`, the MSVC
branch of `src-tauri/build.rs`) is never checked.

## Scope

- Add `cargo clippy --workspace --all-targets -- -D warnings` to the Windows job, reusing its cache.
  `fmt` is platform-independent; add it only if it is cheap there.
- Fix nothing in Rust yourself. If clippy fails on Windows, report the warnings so `backend` can fix
  them in a separate commit before this lands.
- Keep the job time reasonable (note the before/after duration).

## Done when

- the Windows CI job runs clippy with `-D warnings` and is green on `main`
- one `ci:` commit
