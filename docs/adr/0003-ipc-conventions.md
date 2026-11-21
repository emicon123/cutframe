# ADR-0003: IPC conventions

- **Status:** Proposed
- **Date:** 2026-11-21

## Context

ADR-0001 chooses typed IPC through `tauri-specta`, and CLAUDE.md requires that commands and events be
defined in Rust and `ui/src/bindings.ts` be generated. The stage-0 code (`src-tauri/src/ipc/`)
already follows consistent patterns. These patterns are not written down, and several decisions are
missing or only implied:

- `ipc_builder()` calls `.dangerously_cast_bigints_to_number()`. This turns **every** 64-bit integer
  into a JS `number`, and the only note on its safe range is a doc comment on `Time`.
- Undo labels (`Command::label() -> &'static str`) and `IpcError` messages are English text that the
  UI shows as it is, although a Polish UI locale is planned (CLAUDE.md).
- `Document::revision` starts at 0 for every new `Document`, and the UI drops any payload whose
  revision is lower than the one it holds. After a project is opened or created, the UI would ignore
  updates.
- There is no convention for long-running work, which import, thumbnails, proxies, export and AI
  all need.

## Decision

1. **Commands.** In Rust, names are `snake_case` verb-object (`add_track`, `set_preview_rect`). In TS
   they are camelCase, as generated. One command maps to one user intent. Handlers stay thin:
   validate, build an `app-core` command, call `AppState::edit`, return. Every command and event is
   registered in `ipc_builder()` and nowhere else.
2. **Types.** IPC payloads are view types (`*View`, `ProjectState`) in `src-tauri/src/ipc/types.rs`,
   not the model structs. Small value types (`Time`, ids, `FrameRate`, `TrackKind`,
   `ProjectSettings`) may derive `specta::Type` in `app-core` behind the `specta` feature. Fields are
   camelCase.
3. **Numbers.** `Time` (an `i64` in flicks) is sent as a JS `number`. This is exact for
   |t| < 2^53 flicks, about 147 days. Commands that accept a `Time` reject values outside ±2^53
   with `IpcError::InvalidEdit`. No other 64-bit integer crosses IPC unless it provably stays below
   2^53 (for example revisions and counts). Hashes, byte sizes and similar values are sent as
   strings. Model quantities that must be exact (time, frame rate) are integers or rationals and
   never `f64` seconds. Continuous properties (opacity, volume, position) may be `f32`/`f64`.
   Constants the UI needs (`FLICKS_PER_SECOND`) are exported into `bindings.ts` with
   `Builder::constant`, not copied by hand.
4. **Errors.** Every fallible command returns `Result<T, IpcError>`. `IpcError` is a tagged enum
   serialized as `{ kind, message }`. `kind` is stable API: the UI branches on it and translates it,
   and a variant is never renamed. `message` is English diagnostic text for logs and a fallback
   display. It is not localized.
5. **No user-facing text from Rust.** Rust sends stable keys, and the UI turns them into words
   (`ui/src/i18n`). History labels become a `HistoryLabel` enum (for example `"addTrack"`) instead
   of `&'static str` text. Job stages and error kinds follow the same rule.
6. **State sync.** After every model change Rust emits `project-changed` with the full
   `ProjectState`. This happens after every edit, undo, redo, gesture update (ADR-0006) and load. The
   `revision` is **monotonic for the app session**. It lives in `AppState`, not in `Document`, and is
   never reset when a project is opened, created or closed. The UI keeps the payload with the highest
   revision, so stale or reordered events are harmless.

   **`ProjectChanged` benchmark and budget.** A benchmark (a follow-up task for `backend`) measures
   one edit on a generated 1,000-clip, 20-track project. The measured path is `Document::execute` +
   building `ProjectState` + serializing it to JSON, on the Windows reference machine, a mid-range
   laptop with an iGPU. The budget is **≤ 4 ms per edit, payload ≤ 1 MB**. ADR-0004 (edit cost) and
   ADR-0006 (cost per gesture update) refer to this budget. Full snapshots stay the protocol while
   the budget holds. If it is exceeded, cheaper `Document` cloning comes first (ADR-0006). Patches
   need a new ADR.
7. **Events.** A struct name in PascalCase becomes a kebab-case event name (the tauri-specta
   default, e.g. `ProjectChanged` → `project-changed`, `PlaybackTick` → `playback-tick`,
   `JobProgress` → `job-progress`). Docs refer to an event by either form. Payloads are typed.
   High-frequency events
   are throttled where they are produced: `playback-tick` to at most 60 Hz and `job-progress` to at
   most 10 Hz per job.
8. **Long-running work.** Anything that can take more than about 100 ms runs as a job:

   ```rust
   #[derive(Clone, Copy, Serialize, Deserialize, specta::Type)]
   pub struct JobId(pub Uuid); // UUID v7

   // The command returns at once with the id.
   fn import_media(paths: Vec<String>) -> Result<JobId, IpcError>;
   fn cancel_job(id: JobId) -> Result<(), IpcError>;

   // Events
   pub struct JobProgress { pub job_id: JobId, pub stage: JobStage /* key */, pub progress: Option<f32> /* 0..=1 */ }
   pub struct JobFinished { pub job_id: JobId, pub result: Result<(), IpcError> }
   ```

   If a job changes the model (for example by adding imported media), it applies a normal `app-core`
   command when it finishes, so the change is undoable and emits `project-changed`.
9. **Binary data never goes through JSON.** It goes over `media://` routes such as
   `/<kind>/<id>?<params>`. Routes address resources by id. They never take raw file-system paths,
   so the webview cannot read arbitrary files.
10. **Fire-and-forget UI geometry.** Idempotent notifications such as `set_preview_rect` and
    `set_preview_visible` may return `()` without a `Result`.

## Rationale

- Writing down the patterns that already exist costs nothing and keeps three agents consistent.
- A session-wide revision fixes the open/new-project bug before it is written, and keeps the UI's
  simple rule ("keep the highest revision").
- Sending keys instead of text keeps localization in one place (`ui/src/i18n`) and keeps `app-core`
  free of presentation concerns.
- Full snapshots are the simplest correct sync. The trigger for patches is measured, not guessed.

Rejected:
- **BigInt for `Time` in TS:** exact at any range, but every arithmetic site and canvas call would
  need conversions, and timelines never get near 2^53 flicks.
- **JSON Patch / diff events now:** more code and more ordering bugs before there is a measured need.
- **String errors:** the UI cannot branch on them or translate them.
- **Async (thread-pool) commands for model edits:** they could run out of invoke order. Model edits
  stay synchronous handlers on the main thread (ADR-0004 §3). `async` commands are only for work
  that does not touch the `Document`.

## Consequences and risks

- `Command::label()` and `HistoryState` change from strings to `HistoryLabel`, which changes the
  bindings. The frontend adds the translations.
- The revision counter moves from `Document` into `AppState`, or `Document` takes an initial
  revision.
- `dangerously_cast_bigints_to_number` stays on. Reviewers check rule 3 for every new `u64`/`i64`
  field.
