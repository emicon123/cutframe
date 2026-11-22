# Architecture review: stage-0 baseline

- **Task:** `.claude/tasks/0001-architecture-baseline.task.md`
- **Reviewed:** commit `5b0b7e9` on `main` (2026-11-21). This includes `bf2d92b` (Windows manifest in
  test binaries) and `f5ee53a` (D3D12-only on Windows).
- **Against:** `CLAUDE.md` (architecture rules, hard constraints, quality gates), ADR-0001, and the
  ADRs proposed with this review: [ADR-0002](adr/0002-project-file-format.md),
  [ADR-0003](adr/0003-ipc-conventions.md), [ADR-0004](adr/0004-threading-and-model-ownership.md),
  [ADR-0005](adr/0005-preview-presentation-per-platform.md) and
  [ADR-0006](adr/0006-edit-transactions.md).
- **Method:** read all Rust, UI, CI and config sources. Ran `cargo metadata` to list crate licences
  (520 packages). Nothing was built. Overview: [architecture.md](architecture.md).

**Status key:** **OK** = conforms. **Drift** = the code contradicts a doc. **Gap** = a decision is missing
or undocumented. **Risk** = allowed today but likely to hurt later. **Obsolete** = withdrawn by a
later decision (dated, row kept for history).

**Totals: 49 findings. 16 OK, 4 Drift, 12 Gap, 17 Risk.** Since 2026-11-22 (Windows is the only
platform, ADR-0001 amendment) three are obsolete: F27 and F35 (were Risk) and F36 (was OK).

## Verdict per rule

| CLAUDE.md rule or constraint | Verdict | Findings |
|---|---|---|
| Rule 1: model in Rust, every change a command with undo/redo | **OK.** Missing: transactions and gestures, plus a revision bug | F01, F02, F03, F04, F06 |
| Rule 2: no frames over JSON IPC, native wgpu preview, `media://` for assets | **OK.** Linux frames mode now documented. Spike and airspace still open | F08, F09, F10, F11, F12, F13, F46 |
| Rule 3: one render graph for preview and export | **OK so far.** One renderer serves both targets. No render graph exists yet | F15, F16, F48 |
| Rule 4: real-time audio thread | **Not built yet.** Planned in ADR-0004 | F17 |
| Rule 5: effects are data | **Not built yet.** No schema | F18 |
| Rule 6: time is `i64` flicks in model and IPC | **OK.** The rule for casting 64-bit values was implicit | F19, F20, F21 |
| Rule 7 and hard constraint: LGPL-only FFmpeg, dynamically linked | **OK for crates.** No check yet for the FFmpeg binaries | F25, F26, F27, F28 |
| Rule 8 and hard constraint: no reference-product names or assets | **OK in the product.** The reference product is named in the README and ADR-0001 | F29, F49 |
| Windows 10/11 x64 only. ~~CI green on Linux ARM64~~ (withdrawn 2026-11-22: Windows is the only platform for shipping, development and CI, ADR-0001 amendment) | **OK.** Windows-only code is never linted | F30, F34, F35, F36, F37 |
| NSIS `currentUser`, no admin, self-update | **Partly met.** Installer OK. No updater, no signing | F30, F31, F32 |
| Identifier is a codename, frozen before release | **Open** (user decision) | F33 |
| `bindings.ts` generated, never hand-written | **OK** | F22, F23 |
| Quality gates: insta and golden-frame tests | **Drift.** The tooling is missing | F44 |

## Findings

| ID | Area | Status | Evidence | Recommended action | Owner |
|---|---|---|---|---|---|
| F01 | Model ownership | OK | Mutations go only through `Command` (`crates/app-core/src/command.rs:18-23`, `document.rs:66-77`). IPC handlers delegate (`src-tauri/src/ipc/commands.rs:20-71`) | none | — |
| F02 | No model logic in TS | OK | `ui/src/stores/projectStore.ts:14-20` is a mirror. `features/timeline/geometry.ts` is view math. `Inspector.tsx:52` trims for UX while Rust validates (`command.rs:132-135`) | none | — |
| F03 | Undo granularity | Gap | One `Command` is always one undo entry (`document.rs:16-22, 66-77`). There are no compound edits and no gestures | Accept ADR-0006, then implement `Batch` and the gesture API (`EditPhase`, `cancel_gesture`) before the first drag or slider edit | backend, then frontend |
| F04 | `Document` docs | Drift | Doc comment says edits are copy-on-write via `Arc::make_mut` (`document.rs:11-14`), but `execute` always clones the full project (`document.rs:67`) | Fix the comment. Measure the clone against the ADR-0003 §6 budget | backend |
| F05 | Localization of Rust text | Risk | Undo labels are English `&'static str` (`command.rs:20`, `document.rs:57-63`). `IpcError` messages are shown verbatim (`ipc/error.rs:9-16`, `ui/src/features/editor/TopBar.tsx:14-15`, `stores/appStore.ts:19`) | `HistoryLabel` keys, with the UI translating labels and error kinds (ADR-0003 §4-5, ADR-0006 §5) | backend, then frontend |
| F06 | Revision across projects | Risk | `Document::new` starts the revision at 0 (`document.rs:30`), and the UI drops lower revisions without looking at the project (`projectStore.ts:18`). After open or new project, the UI would ignore updates | Move the revision counter to `AppState` and make it monotonic per session (ADR-0003 §6) | backend |
| F07 | Load validation | Risk | Invariant "sorted, never overlapping" (`crates/app-core/src/model.rs:93`) is not checked by `from_json` (`serialization.rs:26-30`) | `Project::validate` in the load pipeline, with proptest (ADR-0002 §3) | backend |
| F08 | Native preview | OK | Win32 child window plus wgpu surface (`src-tauri/src/preview/windows.rs`, `preview/mod.rs:42-63`, `crates/engine/src/preview.rs:261-304`). The UI only reports a rect (`ui/src/features/preview/PreviewArea.tsx:40-69`) | none | — |
| F09 | Linux frames mode | Gap | RGBA pulled over `media://` (`src-tauri/src/protocol.rs:36-66`, `PreviewArea.tsx:72-118`). ADR-0001 allows a dev mode but did not define it | Accept ADR-0005 (§5: dev-only, same renderer). *2026-11-22: the mode is now legacy and unmaintained (ADR-0005 amendment)* | architect (done) |
| F10 | Compositing spike | Risk | ADR-0001's main risk has no recorded result. `--smoke-test` renders offscreen only (`src-tauri/src/lib.rs:91-115`, `.github/workflows/ci.yml:76-86`). The adapter is picked without `compatible_surface` (`crates/engine/src/render/gpu.rs:42-56`) | Run the ADR-0005 §7 checklist and record it. Extend the smoke test to present into the child window. After attaching, check the adapter can present to the surface, else try the next adapter (follow-up, not in an ADR) | backend, devops |
| F11 | Airspace | Risk | On Linux, DOM overlays draw over the preview without problems, so the bug is invisible on the Pi. `setPreviewVisible` is called only on unmount (`PreviewArea.tsx:64`). *2026-11-22: development is Windows-only now, so the bug is visible during development; the action still stands* | One shared hide/show helper for overlays (ADR-0005 §4). Add a frontend review item | frontend |
| F12 | `media://` threads | Risk | One `std::thread::spawn` per request (`protocol.rs:22-26`) | Bounded pool before thumbnails ship (ADR-0004 §3) | backend |
| F13 | `media://` scheme | OK | Registered (`lib.rs:71`), CSP allows it (`src-tauri/tauri.conf.json:25`), per-platform URL (`ui/src/ipc/media.ts:9-12`) | Add thumbnail and waveform routes by id (ADR-0003 §9) | backend |
| F14 | Idle rendering | Risk | The render loop draws every vsync while visible (`crates/engine/src/preview.rs:194-216`) | Render on change, continuously only during playback (ADR-0004 §3) | backend |
| F15 | One renderer | OK | `TestPattern::encode` serves window and offscreen targets (`preview.rs:340-349`, `render/offscreen.rs:11-14, 57-66`) | Keep this as the render graph grows | — |
| F16 | Render-graph contract | Gap | The engine runs on wall-clock `f32` seconds (`preview.rs:358-360`, `render/test_pattern.rs:9-16`), has no `app-core` dependency (`crates/engine/Cargo.toml`), and nothing reads the snapshot (`src-tauri/src/state.rs:42-45`) | Render-graph design doc: `render(snapshot, Time)`. Shader time relative to the clip. Choice between D3D12VA and D3D11 shared handles for zero-copy decode | architect, then backend |
| F17 | Audio and clock | Gap | No audio code. Playhead is UI-only (`ui/src/stores/playheadStore.ts:3-10`) | ADR-0004 §4-5 (rtrb, atomic sample clock, Rust transport). Audio/playback design doc | architect, then backend |
| F18 | Effects as data | Gap | No `effects/` directory and no `effect.json` schema. The test-pattern shader is engine-internal (`test_pattern.rs:5`), which is fine | Effects design doc with the first effect. Projects reference an effect id and version (ADR-0002 §6). Ship as Tauri resources | architect, then backend, devops |
| F19 | Integer time | OK | `Time(i64)` (`crates/app-core/src/time.rs:14-22`). Flick-aligned `FrameRate` (`time.rs:104-125`). Views use `Time` (`ipc/types.rs:33, 54-55`). UI rounds to integer flicks (`geometry.ts:28-31`, `playheadStore.ts:13`) | none | — |
| F20 | 64-bit over IPC | Gap | `dangerously_cast_bigints_to_number()` applies to every 64-bit integer (`src-tauri/src/lib.rs:43-44`). The range limit was only a comment (`time.rs:16`) | ADR-0003 §3: only `Time` and counters below 2^53 are 64-bit. Reject `Time` outside ±2^53 | backend |
| F21 | Duplicated constant | Risk | `FLICKS_PER_SECOND` copied by hand (`ui/src/lib/time.ts:3-4` vs `time.rs:12`) | Export through tauri-specta `Builder::constant` (ADR-0003 §3) | backend, then frontend |
| F22 | Generated bindings | OK | Header (`ui/src/bindings.ts:1`), export test (`lib.rs:117-123`), CI diff (`ci.yml:52-53`), `pnpm bindings` (`package.json:11`) | none | — |
| F23 | Binding hygiene | Risk | The private `FrameRateRepr` leaks into TS (`bindings.ts:59-64`, `time.rs:132-137`). `.mocks()` (`lib.rs:45`) adds an `@tauri-apps/api/mocks` import to production code (`bindings.ts:6`) | Rename or flatten the repr type. Confirm the mocks import is tree-shaken out of `ui/dist` | backend |
| F24 | Layering | OK | `engine` has no Tauri dependency (`crates/engine/Cargo.toml`). `app-core` has only serde, uuid, thiserror and optional specta (`crates/app-core/Cargo.toml:13-18`) | none | — |
| F25 | Crate licences | OK | `deny.toml:10-28` allows permissive licences only. No GPL-only crate among 520 packages (`r-efi` is MIT OR Apache-2.0 OR LGPL) | none | — |
| F26 | FFmpeg binaries | Gap | cargo-deny cannot see FFmpeg (`deny.toml:13`). There is no provisioning, no runtime check and no notices | Pinned, checksummed LGPL shared build. A CI assertion on `avutil_license()`. LGPL notice and source offer in the installer | devops, backend |
| F27 | Pi FFmpeg | Obsolete (2026-11-22) | The setup script installs Debian's libav* packages (`scripts/setup-dev-linux.sh:13-15`), which are built with GPL options. *Withdrawn: the Pi is no longer a platform (ADR-0001 amendment)* | ~~Code and tests never select GPL-only encoders. Check encoder availability at runtime~~ The encoder rule itself stays, justified by the LGPL-only rule (CLAUDE.md rule 7) and tracked in task 0200 | — |
| F28 | OpenH264 fallback | Resolved 2026-11-22 (`docs/design/export.md`, ADR-0001 amendment: Media Foundation before OpenH264; OpenH264 accepted for private use only) — was: Risk | ADR-0001 names OpenH264 as the fallback encoder. Cisco's patent licence covers only Cisco binaries downloaded at runtime, not bundled ones | Decide in the export design (download on demand or no software fallback) | architect |
| F29 | Branding in product | OK | `productName` and identifier are Cutframe (`tauri.conf.json:3-4`). Own icon (`src-tauri/icons/source.svg`). UI strings in `ui/src/i18n/en.ts` | none | — |
| F30 | Windows ship path | OK | NSIS `currentUser` plus `embedBootstrapper` (`tauri.conf.json:29-43`). CI does install, silent install and smoke test (`ci.yml:73-91`). Draft release on tag with version check (`release.yml:16-37`) | none | — |
| F31 | Self-update | Gap | No `tauri-plugin-updater`, no `plugins.updater` or `createUpdaterArtifacts`. Only a TODO (`release.yml:30-31`) | Updater config, key in GitHub secrets, `latest.json` (devops). Register the plugin and add the update-check UX (backend) | devops, backend |
| F32 | Code signing | Gap | Releases are unsigned (`release.yml:30-36`) | Azure Trusted Signing once the account exists | devops |
| F33 | Release identity | Risk | Codename identifier (`tauri.conf.json:4`). The install directory is derived from it (`ci.yml:81`). Updater key not created yet | Release checklist: freeze the name, identifier, project-file extension (ADR-0002 §1) and updater key before the first public release | devops (user decision) |
| F34 | Windows lint | Risk | clippy and fmt run only on Linux (`ci.yml:47-48` vs `57-71`). `cfg(windows)` code (`preview/windows.rs`, the MSVC branch of `build.rs`) is never linted. *2026-11-22: with the Linux job removed, every check (fmt included) must run in the Windows job* | Add `cargo clippy --workspace --all-targets -- -D warnings` to the Windows job | devops |
| F35 | Pi parity | Obsolete (2026-11-22) | CI uses `ubuntu-24.04-arm` (`ci.yml:19`). The Pi runs Debian 13 (`setup-dev-linux.sh:2`). Low priority. *Withdrawn: Linux ARM64 is no longer a development or CI platform (ADR-0001 amendment)* | ~~Optional Debian 13 arm64 container job~~ none | — |
| F36 | Pi dev path | Obsolete (2026-11-22) | Downlevel limits (`gpu.rs:62-64`), software fallback (`gpu.rs:48-56`), frames mode (`preview/mod.rs:65-69`). *Withdrawn: the Pi dev path is legacy and no longer maintained (ADR-0001 and ADR-0005 amendments)* | none | — |
| F37 | Windows GPU backend | OK | D3D12 only, `WGPU_BACKEND` override (`gpu.rs:32-41`, `f5ee53a`). Matches ADR-0001 and ADR-0005 §1 | none | — |
| F38 | Schema versioning | OK | `CURRENT_SCHEMA_VERSION`, `migrate`, `TooNew`, v1 fixture (`serialization.rs:10-47`, `tests/fixtures/project-v1.json`) | none | — |
| F39 | Project-file decisions | Gap | Undecided: contents, media references, atomic save, backups, validation | Accept ADR-0002 and implement it with open/save | backend |
| F40 | IPC conventions | Gap | Consistent patterns (`ipc/commands.rs`, `error.rs`, `events.rs`) but nothing written down. No job convention | Accept ADR-0003 | backend, frontend |
| F41 | Full-state events | Risk | Every edit builds and emits the whole `ProjectState` on the main thread (`state.rs:51-70`, `ipc/types.rs:58-98`) | `ProjectChanged` benchmark, ≤ 4 ms per edit at 1,000 clips (ADR-0003 §6) | backend |
| F42 | Threading model | Gap | Sync commands run on the main thread. `Mutex<Document>` plus `ArcSwap` (`state.rs:16-22`) are not documented | Accept ADR-0004 | backend |
| F43 | `build.rs` + manifest | OK | `src-tauri/build.rs:1-24`, `src-tauri/windows-app-manifest.xml` (`bf2d92b`). See the section below | Follow-ups below | backend, devops |
| F44 | Test tooling | Drift | CLAUDE.md lists insta snapshots and golden-frame tests. insta is not a dependency. GPU tests assert single pixels (`offscreen.rs:130-161`). `proptest` is declared but unused (`crates/app-core/Cargo.toml:20-21`) | Add insta with a canonical project snapshot (ADR-0002 §2). Build a golden-frame harness together with the render graph | backend |
| F45 | UI stack | OK | React 19, Vite 8, Tailwind 4, Zustand, Radix/shadcn (`ui/package.json`). Canvas 2D timeline (`features/timeline/renderer.ts`) | none | — |
| F46 | Preview visibility | Drift | `set_rect` always uses `SWP_SHOWWINDOW` (`preview/windows.rs:108`) and enables rendering whenever the rect is non-empty (`preview/mod.rs:95-96`). That overrides `set_preview_visible(false)` (`ipc/commands.rs:120-127`). This contradicts ADR-0005 §3 | Track the UI's visibility request separately and show the window only if requested and the rect is non-empty. The UI requests visible on mount | backend, then frontend |
| F47 | Agent prompts | Drift | `backend.md:98, 103, 110` names `app-core/src/migrations/`, `ipc/commands/*.rs` and `project_changed { revision, view }`. `frontend.md:74, 85, 93-94` names `lib/timecode.ts` and `hidePreview`/`showPreview`. None of these exist (actual: `serialization.rs`, `ipc/commands.rs`, `project-changed` with `ProjectState`, `lib/time.ts`, `setPreviewVisible`) | Align the prompts with the code and ADR-0003 | orchestrator |
| F48 | Offscreen allocations | Risk | `render_offscreen` creates a texture and a read-back buffer on every call (`offscreen.rs:27-53`) | Reuse targets per size before export ~~or a faster frames mode~~ (frames mode is legacy since 2026-11-22) | backend |
| F49 | Reference product named in docs | Risk | `README.md:3` and `docs/adr/0001-tech-stack.md:8, 91, 111` name the reference product. Not in the shipped product | Decide whether repo docs may name it. If not, reword README and supersede the wording in ADR-0001 | user decision |

## `src-tauri/build.rs` + `windows-app-manifest.xml` (commit `bf2d92b`)

**Verdict: fits the architecture (F43).**
- The manifest is byte-identical to tauri-build 2.7.6's default `windows-app-manifest.xml` (Common
  Controls v6), so the shipped exe should be unchanged.
- The change additionally embeds the manifest, through the MSVC linker, into the `cargo test`
  binaries and the cdylib. That fixes `STATUS_ENTRYPOINT_NOT_FOUND` in the Windows CI test step.
- Non-MSVC targets keep Tauri's default path.

Follow-ups:
- **`/WX`** makes every linker warning fatal for every cutframe artifact. Revisit it when FFmpeg
  import libraries are linked, because LNK4099-style warnings are common there.
- In CI, check that the release exe has exactly one `RT_MANIFEST` (for example with
  `mt.exe -inputresource`).
- This XML is now the single app manifest. Future entries (`longPathAware`, UTF-8 code page) go
  there, not into tauri-build attributes.

## Prioritised follow-up tasks

**P0: before stage-1 feature work**
1. **backend:** fix preview visibility (F46). Make the revision monotonic per session (F06). Change
   history labels to `HistoryLabel` keys (F05).
   **frontend:** request visible on mount, and translate labels and error kinds.
2. **backend + devops:** run the native-preview spike checklist (ADR-0005 §7) and record the
   results. Extend `--smoke-test` to present into the child window, and check that the adapter can
   present to the surface (F10).
3. **devops:** add clippy (and fmt) to the Windows CI job (F34).
4. **backend:** edit transactions per ADR-0006 (`Batch`, `EditPhase`, `cancel_gesture`) (F03).
   **frontend:** gesture helper that sends one `Update` per frame, `Commit` on pointer-up, and
   `cancel_gesture` on Escape, blur and startup.
5. **user / orchestrator:** accept or amend ADR-0002…0006. Align the agent prompts (F47).

**P1: with media import and playback**
1. **architect:** design docs for media import + decode + `media://` thumbnails, the render graph
   (`render(snapshot, Time)`, zero-copy decode choice) and audio/transport (F16, F17).
2. **backend:** bounded `media://` pool (F12), render on change (F14), `ProjectChanged` benchmark
   (F41), `Project::validate` and atomic save (F07, ADR-0002).
3. **devops + backend:** LGPL FFmpeg provisioning, a licence assertion in CI and at runtime, installer
   notices (F26). Encoder selection that works without GPL codecs (LGPL rule; F27 itself is obsolete).
4. **frontend:** shared preview-occlusion helper and a review checklist item (F11).

**P2: before the first public release**
1. **devops + backend:** updater (F31) and code signing (F32).
2. **user + devops:** freeze the name, identifier, file extension and updater key (F33).
   Decide whether docs may name the reference product (F49).
3. **backend:** export `FLICKS_PER_SECOND` (F21), clean up the `FrameRateRepr` and mocks leaks (F23),
   fix the `Document` comment (F04), add insta and a golden-frame harness (F44), reuse offscreen
   targets (F48), and the `build.rs` follow-ups (F43).
4. **architect:** the export design decides the OpenH264 fallback (F28). The effects design doc
   (F18).
5. ~~**devops:** optional Debian 13 job (F35).~~ Withdrawn 2026-11-22: Linux ARM64 is no longer a
   CI platform (ADR-0001 amendment).
