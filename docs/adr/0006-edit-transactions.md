# ADR-0006: Edit transactions for gestures and compound edits

- **Status:** Accepted
- **Date:** 2026-11-21

## Context

`CLAUDE.md` requires every model change to be an undoable command, and the agent prompts say that a
continuous gesture (dragging a clip, trimming, moving a slider) is one undo step. The code has only
single commands: `app_core::command::Command` (`apply` / `revert`) and `Document::execute` / `undo` /
`redo` (`crates/app-core/src/document.rs`). There is no way to show the model change live during a
drag (the preview is rendered from the model) and still record one history entry. The first timeline
editing features need this, and the shape of the IPC commands depends on it.

## Decision

1. **One user action is one history entry.** Two mechanisms cover this:
   - **Compound command:** `Batch { label, commands: Vec<Box<dyn Command>> }` applies its children in
     order and reverts them in reverse. If a child fails, the children already applied are reverted
     and the batch fails. Use it for discrete multi-step edits such as "delete selection" or "split
     at playhead on all tracks".
   - **Gesture:** for continuous edits.
2. **Gesture model ("replace from base"):**
   - `Document::gesture_update(command)` opens a gesture if none is open and stores
     `base = snapshot()`. Every update applies a **fresh** command to a clone of `base`, built from
     **absolute** parameters (new start time, new parameter value, never deltas), and publishes the
     result. The previous update's command is dropped.
   - `Document::gesture_commit(command)` applies the final command to `base` in the same way and
     pushes exactly one entry onto the undo stack. Because that command was applied to `base`, its
     `revert` restores `base` exactly.
   - `Document::gesture_cancel()` restores `base` and records nothing.
   - Every update increases `revision` and emits `ProjectChanged`, so preview and UI show the
     in-progress state.
3. **Rules while a gesture is open:** only one gesture may be open. `execute` returns
   `CommandError::GestureInProgress`. `undo` or `redo` cancels the gesture and does nothing else.
   An update or commit of a different command kind than the open gesture's is rejected. Saving
   commits nothing and serializes `base`, so the half-finished state is never saved.
4. **IPC shape:** commands that support gestures take a phase argument, and one command cancels:

   ```rust
   #[derive(Clone, Copy, Serialize, Deserialize, specta::Type)]
   #[serde(rename_all = "camelCase")]
   pub enum EditPhase { Update, Commit }

   // Example (the real commands come with their feature designs):
   #[tauri::command] #[specta::specta]
   pub fn set_clip_start(clip: ClipId, start: Time, phase: EditPhase) -> Result<(), IpcError>;

   #[tauri::command] #[specta::specta]
   pub fn cancel_gesture() -> Result<(), IpcError>;
   ```

   `Commit` without an open gesture is a plain `execute`, which covers keyboard nudges and typed
   values. The UI sends at most one `Update` per animation frame, sends `Commit` on pointer-up, and
   sends `cancel_gesture` on Escape, on lost pointer capture and on window blur.
5. **History:** labels are stable `HistoryLabel` keys (for example `moveClip`) that the UI translates
   (ADR-0003 §5). The limit stays at
   `DEFAULT_HISTORY_LIMIT` (500) entries, counting a batch or a gesture as one entry.

## Rationale

- "Replace from base" keeps every history entry a single valid `Command` whose `revert` is exact. It
  needs no coalescing logic, and the work per update (one clone plus one apply) is what `execute`
  already does today.
- Absolute parameters make updates idempotent and order-tolerant, which also fits the revision rule
  of ADR-0003.
- The live state goes through the model, so the engine-rendered preview shows exactly what will be
  committed. There is no second "drag ghost" state path.

Rejected:

- **Merging consecutive commands by label or timing:** fragile, because two separate drags could be
  merged and a slow drag could be split.
- **Accumulating every intermediate step into a batch:** history grows with pointer events, and
  revert cost grows with the length of the gesture.
- **UI-only drag state with one command on drop:** the native preview could not show the change
  live, and model logic would leak into TypeScript.
- **Explicit `begin_gesture` / `end_gesture` commands with a `GestureId`:** one more round trip and
  more state to clean up. The phase argument covers the same cases.

## Consequences and risks

- Every gesture-capable command must be constructible from absolute values. Reviews check this.
- A UI that never sends `Commit` or `cancel_gesture` (for example after a webview reload) leaves a
  gesture open. The next `execute` fails with `GestureInProgress`, so the UI calls `cancel_gesture`
  during startup sync. That is a one-line addition to `startProjectSync`.
- Cloning `base` on every update must stay within the ADR-0003 benchmark budget. If it does not,
  `Arc`-shared tracks (structural sharing) are the next step, without changing this API.
