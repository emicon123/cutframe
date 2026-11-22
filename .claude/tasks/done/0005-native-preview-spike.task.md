# Task: Validate the native Windows preview (ADR-0005 §7 spike)

**Agents:** `backend` (smoke test + adapter check), `devops` (CI wiring), **user** (manual hardware
checks). Run after task 0004 (both edit `ci.yml`) and after task 0003 (preview visibility fix).
**Source:** `docs/architecture-review.md` F10 (P0 #2). This is ADR-0001's main open risk.

## Automated part

- **backend:** extend `--smoke-test` (`src-tauri/src/lib.rs:91-115`) so that on Windows it creates the
  preview child window and presents frames into it (not only offscreen), then exits with a clear
  pass/fail. It must work on WARP (no real GPU in CI).
- **backend:** after the preview surface is attached, check that the chosen adapter can present to it
  (`compatible_surface`, `crates/engine/src/render/gpu.rs:42-56`); if not, try the next adapter and log
  the choice. Keep the D3D12-only rule (ADR-0001).
- **devops:** run the extended smoke test in the Windows CI job (`ci.yml:76-86`).

## Manual part (user, real hardware incl. one hybrid-GPU laptop)

Checklist from ADR-0005 §7. Record each result (pass/fail, machine, GPU, notes):

- [ ] correct z-order over WebView2, no flicker while resizing panels or the window
- [ ] correct position after moving the window between monitors with different scale factors
- [ ] minimize, restore, maximize and full-screen keep the preview aligned
- [ ] hiding for an overlapping dialog works and the preview does not reappear on layout changes
- [ ] 1080p at 60 Hz present rate without dropped frames on an integrated GPU, UI responsive

## Done when

- smoke test presents into the child window and is green in CI
- the architect adds the recorded results to ADR-0005 and sets it to Accepted, or, if the spike fails,
  writes the Plan B ADR (`ICoreWebView2SharedBuffer`, see ADR-0001)
