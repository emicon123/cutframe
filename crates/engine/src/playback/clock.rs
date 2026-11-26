//! The master clock of playback (playback.md §5.1): clock samples published by the audio
//! callback through a seqlock, and the [`ClockSource`] the render thread reads.
//!
//! The audio callback (real-time) is the only writer of a [`ClockCell`]: after every device
//! buffer it publishes which timeline time reaches the DAC and when. [`AudioClock`] extrapolates
//! the audible time from the newest sample with integer arithmetic. Without an output device the
//! feeder drives the same callback at wall-clock speed into a scratch buffer ([`WallClock`]
//! paces it), so the render thread reads one [`AudioClock`] for the whole life of the engine,
//! device or not.
//!
//! Everything here is atomics and integer arithmetic: no allocation, no lock, no system call
//! other than `QueryPerformanceCounter` (through [`Instant`]), so the callback may use it.

use std::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI64, AtomicU8, AtomicU32, AtomicU64, Ordering,
};
use std::sync::{Arc, OnceLock, atomic};
use std::time::Instant;

use app_core::{FLICKS_PER_SECOND, Time};

use crate::transport::TransportShared;

const NANOS_PER_SECOND: i128 = 1_000_000_000;

static PROCESS_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Nanoseconds since a process-wide epoch, monotonic (`QueryPerformanceCounter` on Windows).
///
/// Real-time safe once initialised: no allocation, no lock. Call [`init_monotonic`] (or this
/// function) once outside real-time code first, so the callback never runs the one-time
/// initialisation; the audio engine does that when it starts.
pub fn monotonic_nanos() -> u64 {
    let epoch = PROCESS_EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Fixes the process epoch of [`monotonic_nanos`] (idempotent).
pub fn init_monotonic() {
    PROCESS_EPOCH.get_or_init(Instant::now);
}

/// `nanos x rate` in flicks, rounded towards negative infinity, in `i128` (playback.md §5.1).
pub fn nanos_to_flicks(nanos: i64, rate: i8) -> i64 {
    let flicks = i128::from(nanos) * i128::from(FLICKS_PER_SECOND) * i128::from(rate);
    i64::try_from(flicks.div_euclid(NANOS_PER_SECOND)).unwrap_or(if flicks < 0 {
        i64::MIN
    } else {
        i64::MAX
    })
}

/// How the run of a [`ClockSample`] began, which decides what a reading reports before the
/// run's first sample reaches the DAC (`run_at`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStart {
    /// The first audio of a play session (play, exact seek while playing): no reading until it
    /// is audible (preroll).
    First,
    /// Audio resumed after an underrun: the clock stays frozen at `run_start` until it is
    /// audible.
    Resumed,
    /// The run follows another one without a gap (a loop wrap, a shuttle rate change): until it
    /// is audible, the previous run is still playing, and the reading extrapolates it from its
    /// end (`previous_end` at `previous_rate`) with the previous epoch.
    Follows {
        previous_end: Time,
        previous_rate: i8,
    },
}

/// What the audio callback publishes after a device buffer (playback.md §5.1).
///
/// A *run* is a stretch of real (not silent) samples with one epoch, one rate and no underrun
/// gap. `time`/`at_dac` anchor the newest buffer; `run_start`/`run_at` anchor the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// The audio engine's play session (internal; changes on play, pause, seek and scrub).
    pub session: u32,
    pub seek_gen: u32,
    /// The run's epoch. Changes at every loop wrap, rate change and play start, and never
    /// repeats within an engine, so `(seek_gen, epoch)` names one continuous run.
    pub epoch: u32,
    /// Timeline time of the first real sample of the run in this buffer.
    pub time: Time,
    /// When that sample reaches the DAC: nanoseconds of [`monotonic_nanos`].
    pub at_dac: u64,
    /// Timeline time just after the last real sample written so far (not silence).
    pub valid_until: Time,
    /// Playback rate of the run (-8..=8, never 0).
    pub rate: i8,
    /// Timeline time of the run's first sample, and when it reaches the DAC.
    pub run_start: Time,
    pub run_at: u64,
    pub run_kind: RunStart,
    /// The run ended at `valid_until`: the end of the timeline (or 0 in reverse) without loop.
    /// No more audio follows in this session.
    pub ended: bool,
}

/// What the render thread reads: the audible timeline time now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockReading {
    pub time: Time,
    pub seek_gen: u32,
    /// See [`ClockSample::epoch`].
    pub epoch: u32,
    pub rate: i8,
    /// Playback reached the end of the timeline (forward, no loop) or 0 (reverse): `time` is
    /// where it stopped. The transport pauses on the last frame.
    pub ended: bool,
}

/// The master clock as the render thread sees it (playback.md §5.1).
pub trait ClockSource: Send + Sync {
    /// Timeline time that is audible now. `None` before the first sample of the current play
    /// session reached the DAC (preroll), while paused, and for readings of an older seek
    /// generation.
    fn now(&self) -> Option<ClockReading>;
}

const KIND_FIRST: u8 = 0;
const KIND_RESUMED: u8 = 1;
const KIND_FOLLOWS: u8 = 2;

/// A seqlock holding the newest [`ClockSample`] (playback.md §5.1): sequence counter plus
/// atomics. One writer (the audio callback of the open output) never waits; readers retry on a
/// torn read.
///
/// It also holds the audio engine's current *session*: the main thread starts a new one for
/// every play, pause, seek and scrub, so blocks and clock samples of the previous one are stale
/// at once, before the feeder thread has seen the command.
#[derive(Debug, Default)]
pub struct ClockCell {
    /// Even: stable; odd: a write is in progress; 0: nothing published yet.
    seq: AtomicU32,
    session: AtomicU32,
    seek_gen: AtomicU32,
    epoch: AtomicU32,
    time: AtomicI64,
    at_dac: AtomicU64,
    valid_until: AtomicI64,
    rate: AtomicI8,
    run_start: AtomicI64,
    run_at: AtomicU64,
    run_kind: AtomicU8,
    previous_end: AtomicI64,
    previous_rate: AtomicI8,
    ended: AtomicBool,
    current_session: AtomicU32,
}

impl ClockCell {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current play session.
    pub fn current_session(&self) -> u32 {
        self.current_session.load(Ordering::Acquire)
    }

    /// Starts a new play session and returns it (main thread, through the audio engine).
    pub fn begin_session(&self) -> u32 {
        self.current_session
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    /// Publishes `sample`. Single writer: the callback of the open output. Wait-free.
    pub fn publish(&self, sample: &ClockSample) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        atomic::fence(Ordering::Release);
        let (kind, previous_end, previous_rate) = match sample.run_kind {
            RunStart::First => (KIND_FIRST, 0, 0),
            RunStart::Resumed => (KIND_RESUMED, 0, 0),
            RunStart::Follows {
                previous_end,
                previous_rate,
            } => (KIND_FOLLOWS, previous_end.flicks(), previous_rate),
        };
        let r = Ordering::Relaxed;
        self.session.store(sample.session, r);
        self.seek_gen.store(sample.seek_gen, r);
        self.epoch.store(sample.epoch, r);
        self.time.store(sample.time.flicks(), r);
        self.at_dac.store(sample.at_dac, r);
        self.valid_until.store(sample.valid_until.flicks(), r);
        self.rate.store(sample.rate, r);
        self.run_start.store(sample.run_start.flicks(), r);
        self.run_at.store(sample.run_at, r);
        self.run_kind.store(kind, r);
        self.previous_end.store(previous_end, r);
        self.previous_rate.store(previous_rate, r);
        self.ended.store(sample.ended, r);
        // Skip 0 on wrap-around so "nothing published" stays unambiguous.
        let next = match seq.wrapping_add(2) {
            0 => 2,
            n => n,
        };
        self.seq.store(next, Ordering::Release);
    }

    /// The newest sample, `None` before the first publish.
    pub fn load(&self) -> Option<ClockSample> {
        loop {
            let before = self.seq.load(Ordering::Acquire);
            if before == 0 {
                return None;
            }
            if before & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let r = Ordering::Relaxed;
            let kind = self.run_kind.load(r);
            let previous_end = Time(self.previous_end.load(r));
            let previous_rate = self.previous_rate.load(r);
            let sample = ClockSample {
                session: self.session.load(r),
                seek_gen: self.seek_gen.load(r),
                epoch: self.epoch.load(r),
                time: Time(self.time.load(r)),
                at_dac: self.at_dac.load(r),
                valid_until: Time(self.valid_until.load(r)),
                rate: self.rate.load(r),
                run_start: Time(self.run_start.load(r)),
                run_at: self.run_at.load(r),
                run_kind: match kind {
                    KIND_RESUMED => RunStart::Resumed,
                    KIND_FOLLOWS => RunStart::Follows {
                        previous_end,
                        previous_rate,
                    },
                    _ => RunStart::First,
                },
                ended: self.ended.load(r),
            };
            atomic::fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == before {
                return Some(sample);
            }
        }
    }
}

/// `a` limited to `limit` in the direction of `rate` (forward: at most; reverse: at least).
fn not_past(a: Time, limit: Time, rate: i8) -> Time {
    if rate >= 0 {
        a.min(limit)
    } else {
        a.max(limit)
    }
}

/// `a` limited to `floor` against the direction of `rate` (forward: at least; reverse: at most).
fn not_before(a: Time, floor: Time, rate: i8) -> Time {
    if rate >= 0 {
        a.max(floor)
    } else {
        a.min(floor)
    }
}

/// The clock the audio callback drives (playback.md §5.1):
/// `t = time + (now - at_dac) x rate`, clamped to `valid_until` (an underrun freezes it), never
/// before the start of its run, readings of the current play session and seek generation only.
#[derive(Debug, Clone)]
pub struct AudioClock {
    cell: Arc<ClockCell>,
    transport: Arc<TransportShared>,
}

impl AudioClock {
    pub fn new(cell: Arc<ClockCell>, transport: Arc<TransportShared>) -> Self {
        Self { cell, transport }
    }

    /// The reading at `now` (nanoseconds of [`monotonic_nanos`]; tests pass virtual times).
    pub fn reading_at(&self, now: u64) -> Option<ClockReading> {
        let s = self.cell.load()?;
        if s.session != self.cell.current_session()
            || s.seek_gen != self.transport.seek_generation()
        {
            return None;
        }
        let reading = |time: Time, epoch: u32, rate: i8, ended: bool| ClockReading {
            time,
            seek_gen: s.seek_gen,
            epoch,
            rate,
            ended,
        };
        if now < s.run_at {
            // The run's first sample is not audible yet.
            let ahead = i64::try_from(s.run_at - now).unwrap_or(i64::MAX);
            return match s.run_kind {
                RunStart::First => None,
                RunStart::Resumed => Some(reading(s.run_start, s.epoch, s.rate, false)),
                RunStart::Follows {
                    previous_end,
                    previous_rate,
                } => {
                    let back = Time(nanos_to_flicks(ahead, previous_rate));
                    let time = previous_end - back;
                    Some(reading(time, s.epoch.wrapping_sub(1), previous_rate, false))
                }
            };
        }
        let elapsed = if now >= s.at_dac {
            i64::try_from(now - s.at_dac).unwrap_or(i64::MAX)
        } else {
            -i64::try_from(s.at_dac - now).unwrap_or(i64::MAX)
        };
        let raw = Time(
            s.time
                .flicks()
                .saturating_add(nanos_to_flicks(elapsed, s.rate)),
        );
        let time = not_before(not_past(raw, s.valid_until, s.rate), s.run_start, s.rate);
        let ended = s.ended && time == s.valid_until;
        Some(reading(time, s.epoch, s.rate, ended))
    }
}

impl ClockSource for AudioClock {
    fn now(&self) -> Option<ClockReading> {
        self.reading_at(monotonic_nanos())
    }
}

/// Paces the null output: without an output device the feeder runs the audio callback into a
/// scratch buffer at wall-clock speed, as a device would, so the ring, the clock, loop, end and
/// preroll behave exactly as with sound (playback.md §6.6, "no device": `WallClock`).
///
/// The null output "plays" `lead` ahead of now, like a device latency, so the clock keeps
/// moving smoothly between feeder wake-ups (Windows timers may wake 15.6 ms late).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallClock {
    sample_rate: u32,
    lead_nanos: u64,
    /// Wall time of device frame 0.
    start: u64,
    /// Frames handed out so far.
    frames: u64,
}

/// One chunk the null output plays now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallChunk {
    pub frames: usize,
    /// When the first frame of the chunk is "heard", minus now: the latency to pass to the
    /// callback.
    pub latency_nanos: u64,
}

impl WallClock {
    pub fn new(sample_rate: u32, lead_nanos: u64, now: u64) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            lead_nanos,
            start: now,
            frames: 0,
        }
    }

    /// Starts counting again from `now` (after the null output was idle).
    pub fn reset(&mut self, now: u64) {
        self.start = now;
        self.frames = 0;
    }

    /// The frames due by `now + lead` that were not handed out yet.
    pub fn due(&mut self, now: u64) -> WallChunk {
        let rate = u128::from(self.sample_rate);
        let elapsed = u128::from(now.saturating_sub(self.start) + self.lead_nanos);
        let target = u64::try_from(elapsed * rate / NANOS_PER_SECOND as u128).unwrap_or(u64::MAX);
        let frames = target.saturating_sub(self.frames);
        let first_at = self.start
            + u64::try_from(u128::from(self.frames) * NANOS_PER_SECOND as u128 / rate)
                .unwrap_or(u64::MAX);
        self.frames = self.frames.max(target);
        WallChunk {
            frames: usize::try_from(frames).unwrap_or(usize::MAX),
            latency_nanos: first_at.saturating_sub(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(time: i64, at_dac: u64, valid_until: i64) -> ClockSample {
        ClockSample {
            session: 1,
            seek_gen: 0,
            epoch: 7,
            time: Time(time),
            at_dac,
            valid_until: Time(valid_until),
            rate: 1,
            run_start: Time(time),
            run_at: at_dac,
            run_kind: RunStart::First,
            ended: false,
        }
    }

    fn clock() -> (Arc<ClockCell>, Arc<TransportShared>, AudioClock) {
        let cell = Arc::new(ClockCell::new());
        let transport = Arc::new(TransportShared::new());
        let clock = AudioClock::new(Arc::clone(&cell), Arc::clone(&transport));
        cell.begin_session();
        (cell, transport, clock)
    }

    const SECOND: i64 = FLICKS_PER_SECOND;
    const NS: u64 = 1_000_000_000;

    #[test]
    fn nanos_convert_to_flicks_exactly() {
        assert_eq!(nanos_to_flicks(1_000_000_000, 1), SECOND);
        assert_eq!(nanos_to_flicks(500_000_000, -2), -SECOND);
        // 1 ns is 0.7056 flicks: floor.
        assert_eq!(nanos_to_flicks(1, 1), 0);
        assert_eq!(nanos_to_flicks(-1, 1), -1);
    }

    #[test]
    fn no_reading_before_anything_is_published_or_audible() {
        let (cell, _, clock) = clock();
        assert_eq!(clock.reading_at(0), None);
        cell.publish(&sample(SECOND, 10 * NS, 2 * SECOND));
        assert_eq!(
            clock.reading_at(10 * NS - 1),
            None,
            "preroll: not audible yet"
        );
        let r = clock.reading_at(10 * NS).unwrap();
        assert_eq!((r.time, r.epoch, r.ended), (Time(SECOND), 7, false));
    }

    #[test]
    fn readings_extrapolate_and_clamp_at_valid_until() {
        let (cell, _, clock) = clock();
        let mut s = sample(SECOND, 10 * NS, SECOND + SECOND / 10);
        s.run_start = Time(0);
        s.run_at = 9 * NS;
        cell.publish(&s);
        // Before the newest anchor (a buffer queued ahead): extrapolated backwards.
        assert_eq!(
            clock.reading_at(10 * NS - NS / 2).unwrap().time,
            Time(SECOND - SECOND / 2)
        );
        assert_eq!(
            clock.reading_at(10 * NS + NS / 20).unwrap().time,
            Time(SECOND + SECOND / 20)
        );
        // An underrun: the clock freezes at the last real sample.
        assert_eq!(
            clock.reading_at(20 * NS).unwrap().time,
            Time(SECOND + SECOND / 10)
        );
    }

    #[test]
    fn reverse_rates_count_down_and_clamp() {
        let (cell, _, clock) = clock();
        let mut s = sample(SECOND, 10 * NS, 0);
        s.rate = -2;
        cell.publish(&s);
        assert_eq!(
            clock.reading_at(10 * NS + NS / 4).unwrap().time,
            Time(SECOND / 2)
        );
        assert_eq!(clock.reading_at(11 * NS).unwrap().time, Time::ZERO);
    }

    #[test]
    fn ended_is_reported_at_the_end_only() {
        let (cell, _, clock) = clock();
        let mut s = sample(0, NS, SECOND);
        s.ended = true;
        cell.publish(&s);
        assert!(!clock.reading_at(NS + NS / 2).unwrap().ended);
        let end = clock.reading_at(3 * NS).unwrap();
        assert!(end.ended);
        assert_eq!(end.time, Time(SECOND));
    }

    #[test]
    fn stale_sessions_and_generations_give_no_reading() {
        let (cell, transport, clock) = clock();
        cell.publish(&sample(0, 0, SECOND));
        assert!(clock.reading_at(NS / 2).is_some());
        transport.seek(Time::ZERO);
        assert_eq!(clock.reading_at(NS / 2), None, "older seek generation");
        let mut s = sample(0, 0, SECOND);
        s.seek_gen = 1;
        cell.publish(&s);
        assert!(clock.reading_at(NS / 2).is_some());
        cell.begin_session();
        assert_eq!(clock.reading_at(NS / 2), None, "older play session");
    }

    #[test]
    fn runs_that_follow_or_resume_bridge_until_audible() {
        let (cell, _, clock) = clock();
        // A loop wrap at 2 s: the new run (epoch 8) starts at 0 and is heard at 10 s.
        let mut s = sample(0, 10 * NS, SECOND / 10);
        s.epoch = 8;
        s.run_kind = RunStart::Follows {
            previous_end: Time(2 * SECOND),
            previous_rate: 1,
        };
        cell.publish(&s);
        let before = clock.reading_at(10 * NS - NS / 10).unwrap();
        assert_eq!(
            (before.epoch, before.time),
            (7, Time(2 * SECOND - SECOND / 10))
        );
        let after = clock.reading_at(10 * NS + NS / 20).unwrap();
        assert_eq!((after.epoch, after.time), (8, Time(SECOND / 20)));

        // Resumed after an underrun at 1 s: frozen until the new audio is heard.
        let mut s = sample(SECOND, 20 * NS, SECOND + SECOND / 10);
        s.run_kind = RunStart::Resumed;
        cell.publish(&s);
        assert_eq!(clock.reading_at(19 * NS).unwrap().time, Time(SECOND));
        assert_eq!(
            clock.reading_at(20 * NS + NS / 20).unwrap().time,
            Time(SECOND + SECOND / 20)
        );
    }

    #[test]
    fn the_seqlock_never_returns_a_torn_sample() {
        let cell = Arc::new(ClockCell::new());
        let writer = {
            let cell = Arc::clone(&cell);
            std::thread::spawn(move || {
                for i in 1..=200_000i64 {
                    // Every field derives from `i`, so a torn read is visible.
                    cell.publish(&ClockSample {
                        session: i as u32,
                        seek_gen: i as u32,
                        epoch: i as u32,
                        time: Time(i),
                        at_dac: i as u64,
                        valid_until: Time(i * 2),
                        rate: (i % 7 + 1) as i8,
                        run_start: Time(-i),
                        run_at: i as u64 * 3,
                        run_kind: RunStart::Follows {
                            previous_end: Time(i * 5),
                            previous_rate: (i % 7 + 1) as i8,
                        },
                        ended: i % 2 == 0,
                    });
                }
            })
        };
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let cell = Arc::clone(&cell);
                std::thread::spawn(move || {
                    let mut seen = 0u64;
                    let mut last = 0i64;
                    while last < 200_000 {
                        let Some(s) = cell.load() else { continue };
                        let i = s.time.flicks();
                        assert!(i >= last, "samples never go back");
                        assert_eq!(s.session, i as u32);
                        assert_eq!(s.seek_gen, i as u32);
                        assert_eq!(s.epoch, i as u32);
                        assert_eq!(s.at_dac, i as u64);
                        assert_eq!(s.valid_until, Time(i * 2));
                        assert_eq!(s.rate, (i % 7 + 1) as i8);
                        assert_eq!(s.run_start, Time(-i));
                        assert_eq!(s.run_at, i as u64 * 3);
                        assert_eq!(
                            s.run_kind,
                            RunStart::Follows {
                                previous_end: Time(i * 5),
                                previous_rate: (i % 7 + 1) as i8
                            }
                        );
                        assert_eq!(s.ended, i % 2 == 0);
                        last = i;
                        seen += 1;
                    }
                    seen
                })
            })
            .collect();
        writer.join().unwrap();
        for reader in readers {
            assert!(reader.join().unwrap() > 0);
        }
    }

    #[test]
    fn the_wall_clock_hands_out_frames_at_the_sample_rate() {
        let mut wall = WallClock::new(48_000, 20_000_000, 1_000);
        let first = wall.due(1_000);
        assert_eq!(first.frames, 960, "20 ms lead");
        assert_eq!(first.latency_nanos, 0);
        let next = wall.due(1_000 + 10_000_000);
        assert_eq!(next.frames, 480);
        assert_eq!(
            next.latency_nanos, 10_000_000,
            "frame 960 plays 20 ms after start"
        );
        assert_eq!(wall.due(1_000 + 10_000_000).frames, 0);
        // 10 simulated minutes in uneven steps: exactly 48,000 frames per second, no drift.
        let mut total = 960 + 480;
        let mut now = 1_000 + 10_000_000;
        for step in (0..60_000).map(|i| 7_000_000 + (i % 5) * 1_000_000) {
            now += step;
            total += wall.due(now).frames;
        }
        let expected = (u128::from(now - 1_000 + 20_000_000) * 48_000 / 1_000_000_000) as usize;
        assert_eq!(total, expected);
    }
}
