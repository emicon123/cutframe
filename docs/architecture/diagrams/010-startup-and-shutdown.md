# Diagram 010 — Start-up and shutdown

> **Audience:** `backend`, `devops`. One picture of everything that happens between the process
> starting and the first editable frame, and of the two ways the process can end.

---

## 1. Start-up

The order is fixed by `src-tauri/src/lib.rs`. The preview child window exists before the webview
loads, and the UI never fetches state before it has subscribed to events.

```mermaid
sequenceDiagram
  autonumber
  participant OS as Explorer / CLI
  participant Main as main.rs
  participant Tauri as Tauri builder + setup
  participant Preview as Preview::create
  participant Stores as preferences, recent, recovery, jobs, autosave
  participant UI as React app
  participant Rust as IPC handlers

  OS->>Main: process start, optional .cutframe path
  Main->>Main: tracing init
  Main->>Main: ffmpeg_check — release build exits code 6 when FFmpeg is not the LGPL build
  Main->>Main: debug build writes ui/src/bindings.ts
  alt --smoke-test
    Main->>Main: 90 s watchdog, no single-instance plugin
  else normal
    Main->>Main: single-instance plugin registered first
  end
  Main->>Tauri: dialog + window-state plugins, IPC builder, media:// protocol
  Tauri->>Preview: create the hidden 1x1 Win32 child window on the main thread
  Preview->>Preview: spawn preview-render, create GPU context, pick an adapter that can present
  Tauri->>Stores: load preferences, recent list, recovery lock, job pool, cache, autosave thread
  Tauri->>Tauri: manage AppState, publish the first Arc<Project> snapshot
  Tauri->>UI: load the webview
  UI->>Rust: subscribe to project-changed, quit-requested, open-file-requested, app-info-changed
  UI->>Rust: cancelGesture (a webview reload may have left one open)
  UI->>Rust: getProject, getAppInfo, getPreferences
  UI->>Rust: takeLaunchPath — the project the process was launched with, if any
  Rust->>UI: project-changed, project-changed ...
  UI->>UI: view = editor when a launch path opened, otherwise home
  UI->>Rust: recovery: list orphans, only after the launch path is handled
```

Two failure paths:

- **No GPU adapter.** `Preview::create` returns `None` instead of failing the start; `previewMode`
  is `unavailable` and the app runs with a placeholder in the preview slot.
- **FFmpeg is not LGPL.** Only a release build refuses to start, with exit code 6, before any window
  is created. A debug build logs and continues.

## 2. Shutdown

The window close is the only entry point. A clean project never asks the UI anything, so a hung
webview cannot block exit.

```mermaid
stateDiagram-v2
  [*] --> Running
  Running --> Clean: CloseRequested and !is_dirty and no export
  Running --> Asking: CloseRequested and (is_dirty or exporting) — emit quit-requested, prevent close
  Asking --> Running: Cancel
  Asking --> Clean: Save succeeded then confirm-close
  Asking --> Clean: Don't save then confirm-close
  Asking --> Clean: export cancelled (job-finished) then confirm-close
  Clean --> Destroyed: window destroyed
  Destroyed --> [*]: autosave thread deletes recovery envelope and lock, 2 s budget
  Running --> Crashed: process died
  Crashed --> [*]: lock file left behind — next start offers recovery
```

- `confirm-close` is the only way out of `Asking`; the UI calls it for Save and for Don't save.
- A second launch does not reach this path: the single-instance plugin forwards the arguments,
  focuses the main window and emits `open-file-requested`, which the UI handles like File → Open
  behind the same unsaved-changes prompt.
- `--smoke-test` never reaches `Asking`: it exits with an outcome code from the watchdog or from
  `smoke_test::run`.

## 3. Threads that exist during both

| Thread | Started | Stopped |
|---|---|---|
| main / Tauri event loop | process start | process exit |
| `preview-render` | `Preview::create` in `setup` | `Shutdown` message, joined when the handle drops |
| `media://` handlers (2) | protocol registration | process exit |
| job pool workers | `JobManager` creation | process exit, in-flight jobs are abandoned |
| autosave | `Autosaver` creation | window destroyed; the join has a 2 s budget |
| progress ticker | job pool creation | process exit |

A shutdown never waits on decoding or on `media://`: those threads die with the process.

## Links

- [ADR-0004 — Threading and model ownership](../../adr/0004-threading-and-model-ownership.md)
- [01 — System context](../01-system-context.md)
- [04 — Persistence and lifecycle](../04-persistence-and-lifecycle.md): recovery envelope, lock file
- [07 — Preview and render](../07-preview-and-render.md): `Preview::create`, smoke test
