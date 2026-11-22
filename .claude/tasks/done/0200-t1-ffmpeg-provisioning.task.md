# Task 0200: LGPL FFmpeg provisioning and licence checks

**Tier:** 1 · **Agents:** `devops`, `backend` · **Depends on:** none
**Spec:** `docs/feature-catalog.md` (feature IDs below), `docs/adr/`, `docs/design/`, `CLAUDE.md`

## Why

Every Tier 1 media feature needs FFmpeg. Review F26–F28: nothing guarantees today that shipped DLLs are LGPL-only.

## Features

None directly: this task unblocks the feature tasks that depend on it.

## Scope

- **devops:** pinned, SHA-256-checked LGPL shared FFmpeg 7.x build for Windows x64 in CI and for local Windows dev (setup script); bundle the DLLs with NSIS.
- **backend:** add `rsmpeg` to `engine`; at startup and in CI assert `avutil_license()` is LGPL and log the configure flags; refuse GPL builds in release.
- **backend:** encoder discovery that never selects GPL encoders (`libx264`, `libx265`), even if a non-LGPL FFmpeg build is found at runtime (LGPL-only rule, CLAUDE.md rule 7).
- **backend:** revisit `/WX` in `src-tauri/build.rs` once the import libraries link (review F43).

## Done when

- CI fails if the bundled FFmpeg is not LGPL
- `cargo test` can open a small test clip locally on Windows (and in a manual CI run), using the pinned LGPL DLLs
- design docs/ADRs are in `docs/`, valid Mermaid, no production code; dependent task files are updated if the design changes their scope
- `pnpm check` passes; after any IPC change `pnpm bindings` is re-run and `ui/src/bindings.ts` committed
- architecture rules in `CLAUDE.md` hold (model changes are undoable commands, no frames over JSON IPC, integer flicks, LGPL-only FFmpeg, no reference-product names or assets)
