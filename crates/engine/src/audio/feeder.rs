//! The `audio-feeder` thread and its handle [`AudioEngine`] (playback.md §6.1, §6.2, §6.5, §6.6).
//!
//! The feeder owns the output (the `cpal` stream, created and dropped on this thread, or the
//! null output when there is no device), the audio readers ([`AudioSources`]) and the mixer. It
//! may allocate, lock and do I/O. While playing at 1x it mixes blocks until the ring holds
//! [`TARGET_FILL_BLOCKS`]; at other rates it produces silent blocks whose time still advances
//! (or decreases), so the clock runs; then it sleeps about 2 ms. Block times come from the block
//! index in integer flicks, never accumulated.
//!
//! # Integration (transport and render thread)
//!
//! Every [`AudioEngine`] method is cheap and never blocks: it starts a new *play session* in the
//! [`ClockCell`] (so blocks and clock samples of the previous one are stale at once) and sends a
//! message to the feeder.
//!
//! - **Play / exact seek while playing:** write the transport first (`TransportShared::seek`
//!   for a new generation if the time changed, `set_started(false)`, `set_state(Prerolling)`),
//!   then [`AudioEngine::play`] with the generation `TransportShared::seek_generation()` now
//!   returns. The stream opens if it is closed (20-50 ms, part of preroll).
//! - **Preroll:** the render thread polls [`AudioEngine::ring_ready`] (the ring holds its target
//!   for this session) together with its first frame; when both are ready, or after 300 ms, it
//!   calls `TransportShared::set_started(true)`. The callback then consumes the ring and the
//!   clock starts when the first sample reaches the DAC: [`ClockSource::now`] is `None` until
//!   then.
//! - **Shuttle rate change in the same direction:** [`AudioEngine::set_rate`]: no new session,
//!   the new rate starts after the queued audio (about 85 ms) and the clock follows it exactly.
//! - **Pause:** `set_started(false)`, then [`AudioEngine::pause`]. The stream closes 10 s later.
//! - **Seek while paused:** [`AudioEngine::seek`]; `Scrub` plays a grain.
//! - **Loop toggle:** `TransportShared::set_looping`, then [`AudioEngine::set_looping`] (the
//!   feeder reads the atomic for every block).
//! - **Clock:** [`AudioEngine::clock`] is valid for the life of the engine, with or without a
//!   device. Readings carry `seek_gen` and `epoch`; an epoch never repeats, and changes at loop
//!   wraps, rate changes and play starts. `ended` tells the render thread to pause on the last
//!   frame.
//! - **Device lost:** the engine calls the event sink with [`AudioEvent::DeviceLost`] from the
//!   feeder thread; the transport pauses and emits a tick. The next play reopens the default
//!   device; without one the null output runs (one warning).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use app_core::{Project, Time};
use arc_swap::ArcSwapOption;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use super::callback::{CallbackLoad, OutputCallback, OutputSample};
use super::mix::{PcmProvider, mix_block, mix_scrub};
use super::reader::AudioSources;
use super::ring::{
    AudioBlock, AudioShared, BLOCK_FRAMES, BlockProducer, RING_BLOCKS, TARGET_FILL_BLOCKS, ring,
};
use super::{sample_offset, samples_before};
use crate::playback::clock::{
    AudioClock, ClockCell, ClockSource, WallClock, init_monotonic, monotonic_nanos,
};
use crate::playback::source::ProjectSource;
use crate::transport::{SeekMode, TransportShared};

/// How long the feeder sleeps between fills while playing.
const FEED_INTERVAL: Duration = Duration::from_millis(2);
/// The stream closes this long after the last pause (playback.md §6.6).
const CLOSE_AFTER: Duration = Duration::from_secs(10);
/// A scrub grain (§6.5): 80 ms with 5 ms fades, at most one per 40 ms.
const GRAIN_MS: u32 = 80;
const GRAIN_FADE_MS: u32 = 5;
const GRAIN_INTERVAL: Duration = Duration::from_millis(40);
/// The null output plays this far ahead of now.
const WALL_LEAD_NANOS: u64 = 30_000_000;
/// The null output's rate.
const WALL_SAMPLE_RATE: u32 = 48_000;
/// The lookahead for pre-seeking readers (playback.md §7).
const LOOKAHEAD: Time = Time(app_core::FLICKS_PER_SECOND * 3 / 2);
const PREFETCH_INTERVAL: Duration = Duration::from_millis(100);

/// What the feeder reports to the engine's owner (called on the feeder thread; must not block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEvent {
    /// The output device was removed or invalidated during playback. The stream is closed; the
    /// transport should pause. The next play opens the default device again.
    DeviceLost,
}

/// The open output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputInfo {
    /// False for the null output (no device: no sound, wall-clock timing).
    pub device: bool,
    pub sample_rate: u32,
    pub channels: u16,
    /// The device sample format, e.g. `f32`.
    pub format: &'static str,
    /// Frames per device buffer, if the backend reports it.
    pub buffer_frames: Option<u32>,
}

/// Engine options (tests turn the device off).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOptions {
    /// Open the default output device; false always uses the null output.
    pub use_device: bool,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self { use_device: true }
    }
}

enum Msg {
    Play {
        from: Time,
        rate: i8,
        seek_gen: u32,
        session: u32,
    },
    SetRate(i8),
    Pause,
    Seek {
        time: Time,
        seek_gen: u32,
        session: u32,
        mode: SeekMode,
    },
    Wake,
    Shutdown,
}

/// The audio engine: the feeder thread, its output and the master clock. See the module docs
/// for how the transport and the render thread use it. Dropping it stops the thread and closes
/// the stream.
pub struct AudioEngine {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    cell: Arc<ClockCell>,
    shared: Arc<AudioShared>,
    clock: Arc<AudioClock>,
    info: Arc<ArcSwapOption<OutputInfo>>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine")
            .field("session", &self.cell.current_session())
            .field("output", &self.output())
            .finish_non_exhaustive()
    }
}

impl AudioEngine {
    /// Starts the feeder thread. `snapshot` is the project the preview shows (read for every
    /// block); `events` is called on the feeder thread.
    pub fn new(
        transport: Arc<TransportShared>,
        snapshot: Arc<dyn ProjectSource>,
        events: impl Fn(AudioEvent) + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        Self::with_options(transport, snapshot, events, AudioOptions::default())
    }

    pub fn with_options(
        transport: Arc<TransportShared>,
        snapshot: Arc<dyn ProjectSource>,
        events: impl Fn(AudioEvent) + Send + Sync + 'static,
        options: AudioOptions,
    ) -> std::io::Result<Self> {
        init_monotonic();
        let (tx, rx) = crossbeam_channel::unbounded();
        let cell = Arc::new(ClockCell::new());
        let shared = Arc::new(AudioShared::new());
        let info = Arc::new(ArcSwapOption::empty());
        let clock = Arc::new(AudioClock::new(Arc::clone(&cell), Arc::clone(&transport)));
        let feeder = Feeder {
            rx,
            transport,
            snapshot,
            cell: Arc::clone(&cell),
            shared: Arc::clone(&shared),
            info: Arc::clone(&info),
            events: Box::new(events),
            options,
            output: None,
            sources: AudioSources::new(),
            mode: Mode::Idle,
            epochs: 0,
            pushed: 0,
            grains: Grains::default(),
            close_at: None,
            device_error: Arc::new(AtomicBool::new(false)),
            warned_no_device: false,
            last_prefetch: None,
        };
        let thread = std::thread::Builder::new()
            .name("audio-feeder".into())
            .spawn(move || feeder.run())?;
        Ok(Self {
            tx,
            thread: Some(thread),
            cell,
            shared,
            clock,
            info,
        })
    }

    fn send(&self, msg: Msg) {
        // The feeder only stops on drop, so the channel is open while `self` lives.
        let _ = self.tx.send(msg);
    }

    /// Play (or restart preroll) from `from` at shuttle `rate` for generation `seek_gen`.
    pub fn play(&self, from: Time, rate: i8, seek_gen: u32) {
        let session = self.cell.begin_session();
        self.send(Msg::Play {
            from,
            rate,
            seek_gen,
            session,
        });
    }

    /// Another shuttle speed in the same direction while playing: continues after the queued
    /// audio without a new session.
    pub fn set_rate(&self, rate: i8) {
        self.send(Msg::SetRate(rate));
    }

    /// Pause: no more audio; the stream closes 10 s later.
    pub fn pause(&self) {
        self.cell.begin_session();
        self.send(Msg::Pause);
    }

    /// A seek: `Exact` while playing restarts preroll at `time`, while paused it only stops what
    /// is queued; `Scrub` pauses and plays a grain at `time`.
    pub fn seek(&self, time: Time, seek_gen: u32, mode: SeekMode) {
        let session = self.cell.begin_session();
        self.send(Msg::Seek {
            time,
            seek_gen,
            session,
            mode,
        });
    }

    /// A scrub grain at `time` (playback.md §6.5): `seek(time, seek_gen, SeekMode::Scrub)`.
    pub fn scrub(&self, time: Time, seek_gen: u32) {
        self.seek(time, seek_gen, SeekMode::Scrub);
    }

    /// The loop atomic changed (`TransportShared::set_looping`): an ended feeder resumes.
    pub fn set_looping(&self, on: bool) {
        let _ = on;
        self.send(Msg::Wake);
    }

    /// The master clock, valid for the life of the engine (device or null output).
    pub fn clock(&self) -> Arc<dyn ClockSource> {
        Arc::clone(&self.clock) as Arc<dyn ClockSource>
    }

    /// Preroll: the ring holds its target fill (or the end) for the current session.
    pub fn ring_ready(&self) -> bool {
        self.shared.ready_session.load(Ordering::Acquire) == self.cell.current_session()
    }

    /// Blocks in the ring as the feeder last saw it.
    pub fn ring_fill(&self) -> usize {
        self.shared.fill.load(Ordering::Relaxed) as usize
    }

    /// The lowest ring fill since the previous call, while playing.
    pub fn take_ring_min_fill(&self) -> Option<u32> {
        self.shared.take_min_fill()
    }

    /// Device buffers that found the ring empty during playback.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns()
    }

    /// Callback time relative to buffer time (playback.md §11).
    pub fn callback_load(&self) -> CallbackLoad {
        self.shared.load()
    }

    /// The device's reported output latency of the newest buffer.
    pub fn latency(&self) -> Duration {
        Duration::from_nanos(self.shared.latency_nanos.load(Ordering::Relaxed))
    }

    /// The open output, `None` while the stream is closed.
    pub fn output(&self) -> Option<OutputInfo> {
        self.info.load().as_deref().copied()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the audio feeder panicked");
        }
    }
}

/// Produces the blocks of one run of playback (playback.md §5.3, §6.2): block `i` starts at
/// `start + sample_offset(i x BLOCK_FRAMES x rate)`; the last block before the end is short and
/// ends exactly at the end; with loop the next block starts at 0 in a new epoch.
#[derive(Debug, Clone)]
pub(crate) struct BlockMaker {
    pub session: u32,
    pub seek_gen: u32,
    pub epoch: u32,
    pub rate: i8,
    start: Time,
    index: i64,
    sample_rate: u32,
    ended: bool,
}

impl BlockMaker {
    pub fn new(
        session: u32,
        seek_gen: u32,
        epoch: u32,
        rate: i8,
        from: Time,
        sample_rate: u32,
    ) -> Self {
        Self {
            session,
            seek_gen,
            epoch,
            rate: if rate == 0 { 1 } else { rate },
            start: from,
            index: 0,
            sample_rate,
            ended: false,
        }
    }

    /// Timeline time of the next block.
    pub fn position(&self) -> Time {
        let samples = self.index * BLOCK_FRAMES as i64 * i64::from(self.rate);
        self.start + sample_offset(samples, self.sample_rate)
    }

    /// Continues from the next block at `rate`, in run `epoch`.
    pub fn set_rate(&mut self, rate: i8, epoch: u32) {
        if rate == 0 || rate == self.rate {
            return;
        }
        self.start = self.position();
        self.index = 0;
        self.rate = rate;
        self.epoch = epoch;
    }

    fn wrap(&mut self, epochs: &mut u32) {
        *epochs = epochs.wrapping_add(1);
        self.epoch = *epochs;
        self.start = Time::ZERO;
        self.index = 0;
    }

    fn block(&self, time: Time) -> AudioBlock {
        AudioBlock {
            session: self.session,
            seek_gen: self.seek_gen,
            epoch: self.epoch,
            time,
            end: time,
            rate: self.rate,
            ..AudioBlock::EMPTY
        }
    }

    /// The next block, `None` after the end. `epochs` hands out new epochs for loop wraps.
    pub fn next(
        &mut self,
        project: &Project,
        looping: bool,
        sources: &mut dyn PcmProvider,
        epochs: &mut u32,
    ) -> Option<AudioBlock> {
        let duration = project.duration();
        let forward = self.rate > 0;
        if self.ended {
            // Loop switched on after the end: the next loop starts at 0.
            if !(looping && forward && duration > Time::ZERO) {
                return None;
            }
            self.ended = false;
            self.wrap(epochs);
        }
        let mut start = self.position();
        if forward && start >= duration {
            if looping && duration > Time::ZERO {
                self.wrap(epochs);
                start = Time::ZERO;
            } else {
                self.ended = true;
                return Some(AudioBlock {
                    last: true,
                    ..self.block(start)
                });
            }
        } else if !forward && start <= Time::ZERO {
            self.ended = true;
            return Some(AudioBlock {
                last: true,
                ..self.block(start)
            });
        }
        let speed = i64::from(self.rate.unsigned_abs());
        let remaining = if forward { duration - start } else { start };
        let to_end = (samples_before(remaining, self.sample_rate) + speed - 1) / speed;
        let frames = to_end.min(BLOCK_FRAMES as i64);
        let at_end = frames == to_end;
        let end = match (at_end, forward) {
            (true, true) => duration,
            (true, false) => Time::ZERO,
            (false, _) => start + sample_offset(frames * i64::from(self.rate), self.sample_rate),
        };
        let mut block = AudioBlock {
            end,
            frames: frames as u16,
            last: at_end && !(looping && forward),
            ..self.block(start)
        };
        if self.rate == 1 {
            mix_block(
                project,
                start,
                frames as usize,
                self.sample_rate,
                sources,
                &mut block.samples,
            );
        }
        self.index += 1;
        if block.last {
            self.ended = true;
        }
        Some(block)
    }
}

/// The scrub grain throttle (§6.5): at most one per 40 ms, none for a move of less than one
/// project frame.
#[derive(Debug, Default)]
struct Grains {
    pending: Option<(Time, u32, u32)>,
    last_at: Option<Instant>,
    last_time: Option<Time>,
}

impl Grains {
    /// The pending grain, if one is due at `now`; a move of less than `frame` from the last
    /// grain is dropped.
    fn take(&mut self, now: Instant, frame: Time) -> Option<(Time, u32, u32)> {
        let (time, seek_gen, session) = self.pending?;
        if self
            .last_at
            .is_some_and(|at| now.duration_since(at) < GRAIN_INTERVAL)
        {
            return None;
        }
        self.pending = None;
        if self
            .last_time
            .is_some_and(|last| (time - last).flicks().abs() < frame.flicks())
        {
            return None;
        }
        self.last_at = Some(now);
        self.last_time = Some(time);
        Some((time, seek_gen, session))
    }

    /// Grains are being played (the null output keeps running for them).
    fn active(&self, now: Instant) -> bool {
        self.pending.is_some()
            || self
                .last_at
                .is_some_and(|at| now.duration_since(at) < Duration::from_millis(150))
    }
}

/// A scrub grain at `time`: 80 ms of the mix with 5 ms fades, as blocks tagged `grain`.
pub(crate) fn make_grain(
    project: &Project,
    time: Time,
    seek_gen: u32,
    session: u32,
    sample_rate: u32,
    sources: &mut dyn PcmProvider,
) -> Vec<AudioBlock> {
    let frames = (sample_rate * GRAIN_MS / 1000) as usize;
    let fade = ((sample_rate * GRAIN_FADE_MS / 1000) as usize).min(frames / 2);
    let mut pcm = vec![0.0f32; 2 * frames];
    mix_scrub(project, time, frames, sample_rate, sources, &mut pcm);
    for i in 0..fade {
        let gain = i as f32 / fade as f32;
        for j in [
            2 * i,
            2 * i + 1,
            2 * (frames - 1 - i),
            2 * (frames - 1 - i) + 1,
        ] {
            pcm[j] *= gain;
        }
    }
    pcm.chunks(2 * BLOCK_FRAMES)
        .enumerate()
        .map(|(i, chunk)| {
            let start = time + sample_offset((i * BLOCK_FRAMES) as i64, sample_rate);
            let n = chunk.len() / 2;
            let mut block = AudioBlock {
                session,
                seek_gen,
                time: start,
                end: start + sample_offset(n as i64, sample_rate),
                frames: n as u16,
                grain: true,
                ..AudioBlock::EMPTY
            };
            block.samples[..chunk.len()].copy_from_slice(chunk);
            block
        })
        .collect()
}

enum Mode {
    Idle,
    Playing(BlockMaker),
}

enum OutputKind {
    Device {
        // Dropping it stops the callback; never moved off the feeder thread.
        _stream: cpal::Stream,
    },
    Null {
        callback: Box<OutputCallback>,
        pacer: WallClock,
        scratch: Vec<f32>,
    },
}

struct Output {
    producer: BlockProducer,
    sample_rate: u32,
    kind: OutputKind,
}

struct Feeder {
    rx: Receiver<Msg>,
    transport: Arc<TransportShared>,
    snapshot: Arc<dyn ProjectSource>,
    cell: Arc<ClockCell>,
    shared: Arc<AudioShared>,
    info: Arc<ArcSwapOption<OutputInfo>>,
    events: Box<dyn Fn(AudioEvent) + Send + Sync>,
    options: AudioOptions,
    output: Option<Output>,
    sources: AudioSources,
    mode: Mode,
    /// Hands out epochs; never repeats.
    epochs: u32,
    /// Blocks of the current run pushed (preroll target).
    pushed: usize,
    grains: Grains,
    close_at: Option<Instant>,
    device_error: Arc<AtomicBool>,
    warned_no_device: bool,
    last_prefetch: Option<Instant>,
}

impl Feeder {
    fn run(mut self) {
        raise_thread_priority();
        if self.options.use_device {
            warm_up_device();
        }
        loop {
            let first = match self.timeout() {
                None => match self.rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return,
                },
                Some(timeout) => match self.rx.recv_timeout(timeout) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
            };
            let messages: Vec<Msg> = first.into_iter().chain(self.rx.try_iter()).collect();
            for msg in messages {
                if !self.handle(msg) {
                    return;
                }
            }
            self.check_device();
            self.produce();
            self.pump_null_output();
            self.close_if_idle();
        }
    }

    fn active(&self) -> bool {
        matches!(self.mode, Mode::Playing(_)) || self.grains.active(Instant::now())
    }

    fn timeout(&self) -> Option<Duration> {
        if self.active() {
            return Some(FEED_INTERVAL);
        }
        self.close_at
            .map(|at| at.saturating_duration_since(Instant::now()))
    }

    fn next_epoch(&mut self) -> u32 {
        self.epochs = self.epochs.wrapping_add(1);
        self.epochs
    }

    fn sample_rate(&self) -> u32 {
        self.output
            .as_ref()
            .map_or(WALL_SAMPLE_RATE, |o| o.sample_rate)
    }

    /// Handles one message; false on shutdown.
    fn handle(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Play {
                from,
                rate,
                seek_gen,
                session,
            } => self.start(from, rate, seek_gen, session),
            Msg::SetRate(rate) => {
                let epoch = self.epochs.wrapping_add(1);
                if let Mode::Playing(maker) = &mut self.mode
                    && maker.rate != rate
                    && rate != 0
                {
                    self.epochs = epoch;
                    maker.set_rate(rate, epoch);
                }
            }
            Msg::Pause => {
                self.mode = Mode::Idle;
                self.grains.pending = None;
                self.close_at = Some(Instant::now() + CLOSE_AFTER);
            }
            Msg::Seek {
                time,
                seek_gen,
                session,
                mode: SeekMode::Exact,
            } => match &self.mode {
                Mode::Playing(maker) => {
                    let rate = maker.rate;
                    self.start(time, rate, seek_gen, session);
                }
                Mode::Idle => self.grains.pending = None,
            },
            Msg::Seek {
                time,
                seek_gen,
                session,
                mode: SeekMode::Scrub,
            } => {
                self.mode = Mode::Idle;
                self.open_output();
                self.grains.pending = Some((time, seek_gen, session));
                self.close_at = Some(Instant::now() + CLOSE_AFTER);
            }
            Msg::Wake => {}
            Msg::Shutdown => return false,
        }
        true
    }

    fn start(&mut self, from: Time, rate: i8, seek_gen: u32, session: u32) {
        self.open_output();
        let epoch = self.next_epoch();
        let sample_rate = self.sample_rate();
        self.mode = Mode::Playing(BlockMaker::new(
            session,
            seek_gen,
            epoch,
            rate,
            from,
            sample_rate,
        ));
        self.pushed = 0;
        self.close_at = None;
        self.grains.pending = None;
        self.last_prefetch = None;
        self.shared.take_min_fill();
    }

    /// Opens the default device if the output is closed (or the null output without one), and
    /// restarts the null output's pacing.
    fn open_output(&mut self) {
        if self.device_error.swap(false, Ordering::AcqRel) {
            self.output = None;
        }
        if self.output.is_none() {
            let opened = if self.options.use_device {
                open_device(
                    &self.transport,
                    &self.cell,
                    &self.shared,
                    &self.device_error,
                )
            } else {
                Err("the device is off".to_owned())
            };
            let (output, info) = match opened {
                Ok(device) => {
                    self.warned_no_device = false;
                    tracing::info!(output = ?device.1, "audio output opened");
                    device
                }
                Err(err) => {
                    if self.options.use_device && !self.warned_no_device {
                        tracing::warn!(%err, "no audio output; playing without sound");
                        self.warned_no_device = true;
                    }
                    self.null_output()
                }
            };
            self.info.store(Some(Arc::new(info)));
            self.output = Some(output);
        }
        if let Some(Output {
            kind: OutputKind::Null { pacer, .. },
            ..
        }) = &mut self.output
        {
            pacer.reset(monotonic_nanos());
        }
    }

    fn null_output(&self) -> (Output, OutputInfo) {
        let (producer, consumer) = ring();
        let callback = OutputCallback::new(
            consumer,
            Arc::clone(&self.transport),
            Arc::clone(&self.cell),
            Arc::clone(&self.shared),
            2,
            WALL_SAMPLE_RATE,
        );
        let output = Output {
            producer,
            sample_rate: WALL_SAMPLE_RATE,
            kind: OutputKind::Null {
                callback: Box::new(callback),
                pacer: WallClock::new(WALL_SAMPLE_RATE, WALL_LEAD_NANOS, monotonic_nanos()),
                scratch: vec![0.0; 2 * 1024],
            },
        };
        let info = OutputInfo {
            device: false,
            sample_rate: WALL_SAMPLE_RATE,
            channels: 2,
            format: "f32",
            buffer_frames: None,
        };
        (output, info)
    }

    fn check_device(&mut self) {
        let is_device = matches!(
            self.output,
            Some(Output {
                kind: OutputKind::Device { .. },
                ..
            })
        );
        if !is_device || !self.device_error.load(Ordering::Acquire) {
            return;
        }
        self.device_error.store(false, Ordering::Release);
        tracing::warn!("the audio device was lost; closing the stream");
        self.output = None;
        self.info.store(None);
        if matches!(self.mode, Mode::Playing(_)) {
            self.mode = Mode::Idle;
            (self.events)(AudioEvent::DeviceLost);
        }
    }

    fn produce(&mut self) {
        let Some(output) = &mut self.output else {
            return;
        };
        let sample_rate = output.sample_rate;
        let mut fill = RING_BLOCKS - output.producer.slots();
        if let Mode::Playing(maker) = &mut self.mode {
            // Only once preroll is over: before that the ring is still filling from empty, and
            // the minimum would always read 0 (playback.md §11 `ring_min_fill`).
            if self.transport.started() {
                self.shared.record_fill(fill as u32);
            }
            let project = self.snapshot.project();
            let looping = self.transport.looping();
            while fill < TARGET_FILL_BLOCKS {
                let Some(block) =
                    maker.next(&project, looping, &mut self.sources, &mut self.epochs)
                else {
                    break;
                };
                let ends = block.last;
                if output.producer.push(block).is_err() {
                    break;
                }
                fill += 1;
                self.pushed += 1;
                if self.pushed >= TARGET_FILL_BLOCKS || ends {
                    self.shared
                        .ready_session
                        .store(maker.session, Ordering::Release);
                }
            }
            let due = self
                .last_prefetch
                .is_none_or(|at| at.elapsed() >= PREFETCH_INTERVAL);
            if maker.rate == 1 && due {
                self.last_prefetch = Some(Instant::now());
                self.sources
                    .prefetch(&project, maker.position(), LOOKAHEAD, sample_rate);
            }
        }
        let frame = self.snapshot.project().settings.frame_rate.frame_duration();
        if let Some((time, seek_gen, session)) = self.grains.take(Instant::now(), frame) {
            let project = self.snapshot.project();
            let blocks = make_grain(
                &project,
                time,
                seek_gen,
                session,
                sample_rate,
                &mut self.sources,
            );
            if output.producer.slots() >= blocks.len() {
                for block in blocks {
                    let _ = output.producer.push(block);
                }
            }
        }
    }

    /// Without a device: plays the due frames into a scratch buffer at wall-clock speed.
    fn pump_null_output(&mut self) {
        if !self.active() {
            return;
        }
        let Some(Output {
            sample_rate,
            kind:
                OutputKind::Null {
                    callback,
                    pacer,
                    scratch,
                },
            ..
        }) = &mut self.output
        else {
            return;
        };
        let now = monotonic_nanos();
        let chunk = pacer.due(now);
        let mut done = 0;
        while done < chunk.frames {
            let n = (chunk.frames - done).min(scratch.len() / 2);
            let offset = (done as u128 * 1_000_000_000 / u128::from(*sample_rate)) as u64;
            callback.process_at(&mut scratch[..2 * n], chunk.latency_nanos + offset, now);
            done += n;
        }
    }

    fn close_if_idle(&mut self) {
        let Some(at) = self.close_at else { return };
        if self.active() || Instant::now() < at {
            return;
        }
        self.close_at = None;
        if self.output.take().is_some() {
            tracing::debug!("audio output closed after the idle timeout");
        }
        self.info.store(None);
        self.sources.close_all();
    }
}

/// The feeder runs above normal priority (Windows), so decode and build load on the other cores
/// does not starve the ring (playback.md §11: no underruns while 4K decode loads the CPU).
fn raise_thread_priority() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: `GetCurrentThread` returns a pseudo handle that is always valid for the
        // calling thread and needs no closing.
        let result = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
        if let Err(err) = result {
            tracing::debug!(%err, "cannot raise the audio feeder's priority");
        }
    }
}

/// Looks the default device and its format up once without opening a stream: the first lookup
/// in a process initialises the Windows audio device enumerator (about 250 ms on the reference
/// laptop), which would otherwise delay the first play. No audio session is held afterwards.
fn warm_up_device() {
    let started = Instant::now();
    let found = cpal::default_host()
        .default_output_device()
        .map(|device| device.default_output_config().is_ok());
    tracing::debug!(?found, elapsed = ?started.elapsed(), "audio device lookup warmed up");
}

/// Opens the default output device in shared mode with its mix format and default buffer size.
fn open_device(
    transport: &Arc<TransportShared>,
    cell: &Arc<ClockCell>,
    shared: &Arc<AudioShared>,
    device_error: &Arc<AtomicBool>,
) -> Result<(Output, OutputInfo), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device".to_owned())?;
    let supported = device
        .default_output_config()
        .map_err(|err| format!("no output configuration: {err}"))?;
    let config = supported.config();
    let (producer, consumer) = ring();
    let callback = OutputCallback::new(
        consumer,
        Arc::clone(transport),
        Arc::clone(cell),
        Arc::clone(shared),
        usize::from(config.channels),
        config.sample_rate,
    );
    let flag = Arc::clone(device_error);
    // Runs on the stream's thread: only an atomic store.
    let on_error = move |err: cpal::Error| {
        use cpal::ErrorKind as K;
        if !matches!(err.kind(), K::Xrun | K::RealtimeDenied | K::DeviceChanged) {
            flag.store(true, Ordering::Release);
        }
    };
    let format = supported.sample_format();
    let (stream, name) = match format {
        cpal::SampleFormat::F32 => (build::<f32>(&device, config, callback, on_error)?, "f32"),
        cpal::SampleFormat::I16 => (build::<i16>(&device, config, callback, on_error)?, "i16"),
        cpal::SampleFormat::I32 => (build::<i32>(&device, config, callback, on_error)?, "i32"),
        cpal::SampleFormat::U16 => (build::<u16>(&device, config, callback, on_error)?, "u16"),
        other => return Err(format!("unsupported device sample format {other:?}")),
    };
    stream
        .play()
        .map_err(|err| format!("cannot start the stream: {err}"))?;
    let info = OutputInfo {
        device: true,
        sample_rate: config.sample_rate,
        channels: config.channels,
        format: name,
        buffer_frames: stream.buffer_size().ok(),
    };
    let output = Output {
        producer,
        sample_rate: config.sample_rate,
        kind: OutputKind::Device { _stream: stream },
    };
    Ok((output, info))
}

fn build<T: OutputSample + cpal::SizedSample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut callback: OutputCallback,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String> {
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| callback.process(out, info),
            on_error,
            None,
        )
        .map_err(|err| format!("cannot open the stream: {err}"))
}

#[cfg(test)]
mod tests;
