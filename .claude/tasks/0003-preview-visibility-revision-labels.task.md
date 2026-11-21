# Task: Preview visibility, session revision and translatable labels

**Agents:** `backend` first (IPC change + `pnpm bindings`), then `frontend` against the new bindings.
**Source:** `docs/architecture-review.md` F46, F06, F05 (P0 #1). Design: ADR-0003 §4-6, ADR-0005 §3,
ADR-0006 §5. No new design doc needed.
**Touches:** `crates/app-core/src/{command,document}.rs`, `src-tauri/src/{state.rs,ipc/*,preview/*}`,
`ui/src/{stores,features/editor,features/preview,i18n}`. Must not run in parallel with task 0006.

## 1. Preview visibility (F46, Drift)

- `set_rect` in `src-tauri/src/preview/windows.rs:108` always passes `SWP_SHOWWINDOW`, and
  `preview/mod.rs:95-96` enables rendering for any non-empty rect. That overrides
  `set_preview_visible(false)` (`ipc/commands.rs:120-127`), contradicting ADR-0005 §3.
- **backend:** keep the UI's visibility request as separate state. The window is shown and rendering
  is enabled only when visibility is requested **and** the rect is non-empty. `set_rect` must never
  show a hidden preview.
- **frontend:** `PreviewArea` requests visible on mount and hidden on unmount.
- Test: unit test of the visibility state logic (no window needed).

## 2. Session-wide revision (F06, Risk)

- `Document::new` starts `revision` at 0 (`document.rs:30`) and the UI drops lower revisions
  (`projectStore.ts:18`), so after new/open project the UI would ignore every update.
- **backend:** the revision is monotonic for the app session and lives in `AppState` (or `Document`
  takes an initial revision), per ADR-0003 §6. Replacing the document never lowers it.
- Test: replacing the document yields a revision greater than any revision emitted before.

## 3. Keys instead of English text (F05, Risk)

- Undo labels are English `&'static str` (`command.rs:20`, `document.rs:57-63`) and `IpcError`
  messages are shown verbatim (`ipc/error.rs:9-16`, `TopBar.tsx:14-15`, `appStore.ts:19`).
- **backend:** a `HistoryLabel` key type (e.g. `moveClip`) exported through tauri-specta; history
  state exposes keys, not text. `IpcError.kind` is the stable key; `message` stays for logs only.
- **frontend:** translate history labels and error kinds via `ui/src/i18n/en.ts`; never display
  `IpcError.message` directly. Fallback for an unknown key: a generic string, not the raw key.

## Done when

- `pnpm bindings` regenerated and committed; `pnpm check` passes
- manual check on Windows: hiding the preview (e.g. while a dialog is open) survives a window resize
- one commit per agent, Conventional Commits
