# 09 — Audio and playback clock

> **Audience:** `backend` (engine) and anyone who touches sound, the transport or A/V sync. How
> audio reaches the device, which thread may do what, and how the picture follows the sound. The
> render-thread side of playback (frame choice, preroll, scheduler) is in
> [07](07-preview-and-render.md) §4; the spec is `docs/design/playback.md`.

---

## 1. What exists

Pressing Space plays the timeline with sound (task 0330). The **audio device is the master clock**:
the real-time callback publishes which timeline time reaches the DAC and when, and the render thread
picks the frame for that time. Without an output device the same path runs on wall-clock pacing.

| Piece | Where | State |
|---|---|---|
| Transport atomics | `crates/engine/src/transport.rs` (`TransportShared`) | implemented |
| Transport state machine, `Playback`, audio seam | `crates/engine/src/playback/` ([07](07-preview-and-render.md)) | implemented |
| Clock samples, seqlock, `ClockSource` | `crates/engine/src/playback/clock.rs` | implemented |
| Feeder thread, `AudioEngine` handle, stream lifecycle | `crates/engine/src/audio/feeder.rs` | implemented |
| Ring and `AudioBlock` | `crates/engine/src/audio/ring.rs` | implemented |
| Real-time callback | `crates/engine/src/audio/callback.rs` (`OutputCallback`) | implemented |
| Mixer (shared with export) | `crates/engine/src/audio/mix.rs` (`mix_block`) | implemented |
| Audio readers, `AudioSources` | `crates/engine/src/audio/reader.rs` | implemented |
| Clip speed and time-stretch | playback.md §6.3 | **Target** (0370) |

## 2. Threads and data flow

```mermaid
flowchart LR
  main["main thread: transport commands via Playback"]
  shared["TransportShared (atomics)"]
  feeder["audio-feeder: cpal stream owner, audio readers, mix_block"]
  ring["rtrb ring: 64 AudioBlocks"]
  cb["cpal callback (real-time): copy, convert, publish clock"]
  cell["ClockCell (seqlock)"]
  render["preview-render: clock.now(), frame choice"]
  dev["WASAPI default device (shared mode)"]
  main --> shared
  main -- "AudioCommand" --> feeder
  feeder --> ring
  ring --> cb
  cb --> dev
  cb --> cell
  cell -. "read" .-> render
  shared -. "read" .-> feeder
  shared -. "read" .-> cb
```

| Thread | May | Must not |
|---|---|---|
| `audio-feeder` (above-normal priority on Windows) | allocate, lock, do I/O, decode, mix; create and drop the `cpal` stream (it is not `Send`) | block the main thread |
| cpal callback (WASAPI) | pop blocks, copy, convert channels and sample format, write atomics, read `Instant` | allocate, lock, do I/O or log (CLAUDE.md rule 4) — a counting-allocator test enforces it |
| `preview-render` | read the clock (`ClockSource::now`), set `started` when preroll is done | wait for audio |

## 3. Blocks, ring and callback

- `AudioBlock` is `Copy` and fixed size: 256 stereo `f32` frames plus `seek_gen`, epoch, session,
  timeline `time`, frame count and flags (grain, end). The `rtrb` ring holds `RING_BLOCKS = 64`
  (about 340 ms at 48 kHz); the feeder keeps `TARGET_FILL_BLOCKS = 16` (about 85 ms) queued.
- `OutputCallback::process` writes silence until `started` (except scrub grains), drops blocks of
  an older seek generation or session, converts stereo to the device's channels (mono: average; more
  than two: L, R, zeros) and sample format (`f32`, `i16`, `i32`, `u16`), counts underruns, and
  publishes one `ClockSample` per buffer for its first real frame. Grains never publish the clock.
- Block times come from the block index in integer flicks (14,700 per sample at 48 kHz, 16,000 at
  44.1 kHz; `i128` rounding for other rates), so long playback cannot drift.

## 4. The clock

- `ClockCell` is a seqlock: the callback never waits, the render thread retries a torn read.
- `ClockSource::now()` returns `ClockReading { time, seek_gen, epoch, rate, ended }`: the sample's
  time plus `(now − at_dac) × rate` in integer flicks, clamped to the last real sample, so an underrun
  freezes the picture instead of letting it run ahead of silence. `None` until the first sample of a
  run is audible (preroll).
- `(seek_gen, epoch)` names one continuous run; the epoch changes at every play start, rate change
  and loop wrap and never repeats. An internal session counter separates play after pause from
  stale blocks, so play does not need a new seek generation.
- **No device:** the feeder drives the same callback into a scratch buffer at wall-clock pace (30 ms
  ahead), so loop, end, preroll and underrun behave identically and the render thread keeps one
  clock for the engine's life.

## 5. Stream lifecycle and failures

| Event | Behaviour |
|---|---|
| Feeder start | looks up the default device once (the first lookup costs about 250 ms), opens nothing |
| Play or scrub | opens the default device in shared mode (its mix format and buffer size) if closed |
| 10 s after the last pause or scrub | the stream closes; a new default device is used next time |
| Device removed or invalidated | error flag → `AudioEvent::DeviceLost` → the transport pauses and ticks; next play reopens |
| No output device | null output on wall-clock pacing, one warning |
| Underrun | silence for the gap, clock frozen, `underruns` counted (present-rate log) |
| Offline media or decode error in a voice | that voice is silent, logged once per media |

## 6. Mixer and readers

- `mix_block(project, start, frames, sample_rate, sources, out)` mixes every clip with `audio`
  props on an unmuted track (main-track video clips with audio included; hidden tracks still sound):
  gain `volume.at(local) × linear fade`, evaluated at block start and end and ramped per sample,
  sample-accurate clip boundaries, sum then clamp to −1..=1. Playback calls it at the device rate,
  export (0390) at the project rate with `MIX_CHUNK_FRAMES = 1024`.
- `AudioReader` decodes one audio stream with its own `AVFormatContext`, resamples to stereo `f32`
  with `swresample` (mono plays on both channels at full level), seeks sample-accurately (decoding
  from 200 ms before the target, honouring `origin`). `AudioSources` reuses a reader across split
  points and pre-seeks a second one for jump cuts; at most 16 readers, idle ones close after 5 s.
  Both work on any thread.
- Shuttle rates other than 1x produce silent blocks with advancing (or decreasing) time, so the
  clock still runs. Scrub seeks play an 80 ms grain with 5 ms fades, at most one per 40 ms.

## Links

- [playback design](../design/playback.md), [export design](../design/export.md) (mixer reuse)
- [ADR-0004 — Threading and model ownership](../adr/0004-threading-and-model-ownership.md)
- [07 — Preview and render](07-preview-and-render.md): transport, frame choice, scheduler, ticks
- [05 — IPC contract](05-ipc-contract.md): transport commands and `playback-tick`
