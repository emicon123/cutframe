# Task: Edit transactions and gestures (ADR-0006)

**Agents:** `backend` first (model + IPC + `pnpm bindings`), then `frontend`.
**Gate:** ADR-0006 must be **Accepted** by the user before work starts. Run after task 0003 (same
files: `document.rs`, `command.rs`, `ipc/*`, history labels).
**Source:** `docs/architecture-review.md` F03 (P0 #4). Design: `docs/adr/0006-edit-transactions.md`
(the spec; follow it, do not re-decide it).

## Backend (`crates/app-core`, `src-tauri`)

- `Batch { label, commands }`: applies children in order, reverts in reverse; if a child fails, the
  applied ones are reverted and the batch fails. One history entry.
- Gestures, "replace from base": `Document::gesture_update`, `gesture_commit`, `gesture_cancel` exactly
  as in ADR-0006 §2-3 (absolute parameters, one entry on commit, `GestureInProgress` from `execute`,
  undo/redo cancels an open gesture, mismatched command kind rejected, save serializes `base`).
- Every update bumps the session revision (task 0003) and emits `project-changed`.
- IPC: `EditPhase { Update, Commit }` and `cancel_gesture()` (ADR-0006 §4). Convert the existing edit
  commands that are continuous in the UI (e.g. clip move/trim, numeric inspector fields) to take a
  phase; `Commit` without an open gesture is a plain `execute`.
- Tests (`cargo test -p app-core`): batch rollback on failure, gesture update/commit = one undo entry,
  cancel restores base exactly, undo during a gesture, save during a gesture writes `base`,
  history limit counts a gesture as one entry. Property tests with `proptest` (already declared) for
  execute/undo/redo round-trips are welcome.

## Frontend (`ui/`)

- A gesture helper: at most one `Update` per animation frame, `Commit` on pointer-up,
  `cancel_gesture` on Escape, lost pointer capture, window blur and at startup sync
  (`startProjectSync`).
- Use it in the timeline drag/trim and in inspector sliders/scrubbable fields.
- Vitest for the helper (throttling, commit, cancel paths).

## Done when

- `pnpm bindings` regenerated and committed; `pnpm check` passes
- manual check: dragging a clip and pressing Ctrl+Z undoes the whole drag in one step; Escape during
  a drag restores the original position
- the architect updates ADR-0006 only if implementation forced a change (as a new ADR if it is a
  different decision)

## Outcome (2026-11-22)

Done in 304820b (ADR accepted), 79cc500 (backend) and 3a66d13 (frontend).

Deferred, because no continuous edit command exists yet (the timeline has no clips):
- no IPC command takes a `phase` yet; the first ones (clip move/trim, inspector parameters) come
  with the clip-editing feature design and plug into `ui/src/ipc/gesture.ts`
- the manual check (Ctrl+Z after a clip drag, Escape mid-drag) moves to that feature
- the atomic save (P1) must serialize `Document::committed()`, not `snapshot()`
