# ADR-0004: Threading and model ownership

- **Status:** Proposed
- **Date:** 2026-11-21

## Context

ADR-0001 says that Rust owns the model, that the engine reads copy-on-write snapshots, and that audio
is the master clock. `CLAUDE.md` adds that the audio thread is real-time safe. Which threads exist,
who owns what, and how they talk to each other is not written down. Today the code has:

- **Main thread:** the Tauri event loop, WebView2, the Win32 preview child window
  (`src-tauri/src/preview/windows.rs`) and every IPC command, because all commands are synchronous.
- **`preview-render` thread** (`crates/engine/src/preview.rs`): creates and owns the `GpuContext` and
  takes messages over an unbounded `crossbeam` channel.
- **One OS thread per `media://` request** (`src-tauri/src/protocol.rs`).
- `AppState` (`src-tauri/src/state.rs`): `Mutex<Document>` plus an `ArcSwap<Project>` snapshot that
  nothing reads yet. `engine` does not depend on `app-core`.
- The playhead is UI-only state (`ui/src/stores/playheadStore.ts`).

Decode, audio, export and jobs are next. Their threads must fit one model.

## Decision

1. **Document ownership:** `Document` (model + undo history) stays behind one `Mutex` in `AppState`.
   An edit holds the lock only to apply the command and build the `ProjectState`. Events are emitted
   after the lock is released. No other thread mutates the model.
2. **Snapshots for readers:** after every change the new `Arc<Project>` is published through
   `ArcSwap`. Engine threads read snapshots lock-free and never take the document lock. `engine`
   depends on `app-core` (read-only model types). `app-core` never depends on `engine` or Tauri.
3. **Threads and their rules:**

   | Thread | Owns | Must not |
   |---|---|---|
   | Main (Tauri) | windows, WebView, sync command handlers | block (I/O, decode, render, waiting on a channel) |
   | `preview-render` | preview surfaces and preview renderer | wait on decode; it shows the newest ready frame or holds the last one |
   | Decode workers | FFmpeg contexts, one per active source | touch the model or the UI |
   | Audio callback (`cpal`) | mixing into the device buffer, advancing the clock | allocate, lock, do I/O or log |
   | Audio feeder | decoding and resampling PCM into ring buffers | — |
   | Job workers (bounded pool) | import, thumbnails, waveforms, proxies, export, AI | block the main thread |
   | `media://` handlers (bounded pool) | serving cached assets | do heavy work inline; they hand it to jobs |

   `preview-render` draws only when one of its inputs changes (snapshot revision, playhead, size,
   visibility), and continuously only during playback.

4. **Communication primitives:** `crossbeam` channels for messages to a thread, `ArcSwap` for model
   snapshots, `rtrb` SPSC ring buffers between the audio feeder and the audio callback, and atomics
   for the clock and transport flags. A `Mutex` is never shared with the audio callback.
5. **Clock and transport:** play, pause, seek and the playhead are engine state, not UI state. The
   audio callback advances an atomic sample position, which is the master clock. The render thread
   converts it to `Time` and picks frames from it. Video drops frames when it falls behind. The UI
   playhead becomes a mirror fed by `playback-tick` (ADR-0003), and a scrub sends a seek command.
6. **One GPU device per process.** `wgpu::Device` and `Queue` are created once and shared, because
   they are cloneable handles. Each consumer (preview, export, thumbnail rendering) owns its own
   renderer state and render targets, and all of them run the same render-graph code (CLAUDE.md
   rule 3). Moving the `GpuContext` out of the preview thread is a refactor for when export or GPU
   thumbnails arrive. Until then the preview thread keeps it.

## Rationale

- Edits are small and rare compared to frames, so a short `Mutex` hold is simpler than a dedicated
  model thread and has no visible latency. Readers that must never block (render, audio) use
  snapshots.
- An audio-driven clock is the only way to keep audio glitch-free. Video can drop frames without
  being noticed, audio cannot.
- Sharing one device avoids duplicate uploads and lets decoded GPU textures (zero-copy, ADR-0001) be
  used by both preview and export.

Rejected:

- **A dedicated model thread (actor) for all edits:** adds a channel round trip to every command and
  makes synchronous replies (`add_track` returning the new ID) more complex, without benefit at
  current edit rates.
- **`RwLock<Project>` read by render and audio:** readers would block writers and risk priority
  inversion on the real-time thread.
- **async/tokio for engine work:** real-time and GPU work need dedicated threads with predictable
  scheduling. Tokio stays for async commands only.
- **A separate GPU device for export:** duplicate resources and no texture sharing with decode.

## Consequences and risks

- `Document::execute` clones the whole project for every edit. That is fine at current sizes and must
  be measured with the `ProjectChanged` benchmark (ADR-0003).
- The playhead moves from the UI into Rust when playback is built. `playheadStore` keeps its API, and
  only its source changes.
- The shared GPU device means a device loss affects preview and export together. Recovery is handled
  in one place.
- Thread count must stay bounded on the Pi (4 cores): the pools are sized from
  `std::thread::available_parallelism`.
