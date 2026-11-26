//! The real-time audio callback (playback.md §6.2): copies blocks from the ring into the device
//! buffer, converts stereo to the device's channels and sample format, and publishes the clock.
//!
//! **No allocation, no lock, no I/O, no logging** (CLAUDE.md rule 4). The state is the ring's
//! consumer, one partially read block, two `Arc`s of atomics-only types and plain counters, so a
//! `Mutex` cannot reach it by type. `tests/audio_callback_alloc.rs` runs it under a counting
//! global allocator.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use app_core::Time;

use super::ring::{AudioBlock, AudioShared, BlockConsumer};
use crate::playback::clock::{ClockCell, ClockSample, RunStart, monotonic_nanos};
use crate::transport::TransportShared;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// A device sample format the callback writes (the formats WASAPI shared mode offers).
pub trait OutputSample: Copy + Send + 'static {
    const SILENCE: Self;
    /// `value` in -1..=1 (clamped) as this format.
    fn from_f32(value: f32) -> Self;
}

impl OutputSample for f32 {
    const SILENCE: Self = 0.0;
    fn from_f32(value: f32) -> Self {
        value
    }
}

impl OutputSample for i16 {
    const SILENCE: Self = 0;
    fn from_f32(value: f32) -> Self {
        (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16
    }
}

impl OutputSample for i32 {
    const SILENCE: Self = 0;
    fn from_f32(value: f32) -> Self {
        (f64::from(value.clamp(-1.0, 1.0)) * 2_147_483_647.0).round() as i32
    }
}

impl OutputSample for u16 {
    const SILENCE: Self = 32_768;
    fn from_f32(value: f32) -> Self {
        ((value.clamp(-1.0, 1.0) * 32_767.0).round() as i32 + 32_768) as u16
    }
}

/// Callback time relative to the device time it produced (playback.md §11: at most 10 %).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CallbackLoad {
    /// Total callback time over total buffer time, in percent.
    pub average_percent: f64,
    /// The worst single buffer, in percent of its period.
    pub max_percent: f64,
    pub calls: u64,
}

impl AudioShared {
    /// The callback load since the stream opened.
    pub fn load(&self) -> CallbackLoad {
        let busy = self.busy_nanos.load(Ordering::Relaxed);
        let period = self.period_nanos.load(Ordering::Relaxed);
        CallbackLoad {
            average_percent: if period == 0 {
                0.0
            } else {
                busy as f64 * 100.0 / period as f64
            },
            max_percent: f64::from(self.max_load_ppm.load(Ordering::Relaxed)) / 10_000.0,
            calls: self.calls.load(Ordering::Relaxed),
        }
    }
}

/// The run the clock samples describe (see [`ClockSample`]).
#[derive(Debug, Clone, Copy)]
struct Run {
    /// A run of `session` exists (false before the first real block).
    active: bool,
    session: u32,
    seek_gen: u32,
    epoch: u32,
    rate: i8,
    start: Time,
    start_at: u64,
    /// The run began but none of its frames was written yet: `start_at` is still unknown.
    start_pending: bool,
    kind: RunStart,
    valid_until: Time,
    ended: bool,
    /// The ring ran empty during this run: the next real frame starts a resumed run.
    resume: bool,
}

impl Run {
    const NONE: Run = Run {
        active: false,
        session: 0,
        seek_gen: 0,
        epoch: 0,
        rate: 1,
        start: Time::ZERO,
        start_at: 0,
        start_pending: false,
        kind: RunStart::First,
        valid_until: Time::ZERO,
        ended: false,
        resume: false,
    };
}

/// The device callback's state. [`Self::process`] is what the `cpal` stream calls; tests call it
/// (or [`Self::process_at`] with virtual time) directly.
pub struct OutputCallback {
    consumer: BlockConsumer,
    /// The block being copied; `read` frames of it are done.
    block: AudioBlock,
    read: usize,
    transport: Arc<TransportShared>,
    clock: Arc<ClockCell>,
    shared: Arc<AudioShared>,
    channels: usize,
    sample_rate: u32,
    run: Run,
}

impl std::fmt::Debug for OutputCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputCallback")
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

impl OutputCallback {
    /// A callback for a device with `channels` channels (at least 1) at `sample_rate`.
    pub fn new(
        consumer: BlockConsumer,
        transport: Arc<TransportShared>,
        clock: Arc<ClockCell>,
        shared: Arc<AudioShared>,
        channels: usize,
        sample_rate: u32,
    ) -> Self {
        crate::playback::clock::init_monotonic();
        Self {
            consumer,
            block: AudioBlock::EMPTY,
            read: 0,
            transport,
            clock,
            shared,
            channels: channels.max(1),
            sample_rate: sample_rate.max(1),
            run: Run::NONE,
        }
    }

    /// One device buffer: what the `cpal` stream calls. Measures its own time for the load
    /// statistics.
    pub fn process<T: OutputSample>(&mut self, out: &mut [T], info: &cpal::OutputCallbackInfo) {
        let entered = monotonic_nanos();
        let timestamp = info.timestamp();
        let latency = timestamp
            .playback
            .checked_duration_since(timestamp.callback)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        self.process_at(out, latency, entered);
        let busy = monotonic_nanos().saturating_sub(entered);
        let frames = (out.len() / self.channels) as u128;
        let period = u64::try_from(frames * NANOS_PER_SECOND / u128::from(self.sample_rate))
            .unwrap_or(u64::MAX);
        self.shared.record_call(busy, period);
        self.shared.latency_nanos.store(latency, Ordering::Relaxed);
    }

    /// One device buffer at `now` ([`monotonic_nanos`]), whose first frame reaches the DAC
    /// `latency_nanos` later (playback.md §6.2 steps 1-4).
    pub fn process_at<T: OutputSample>(&mut self, out: &mut [T], latency_nanos: u64, now: u64) {
        let channels = self.channels;
        let frames = out.len() / channels;
        let session = self.clock.current_session();
        let seek_gen = self.transport.seek_generation();
        let started = self.transport.started();
        let stale = |b: &AudioBlock| b.session != session || b.seek_gen != seek_gen;

        // A seek or pause since the last buffer: the rest of the current block is stale.
        if self.read < usize::from(self.block.frames) && stale(&self.block) {
            self.read = usize::from(self.block.frames);
        }
        // Drop stale blocks at the front, also while waiting for preroll.
        while self.consumer.peek().is_ok_and(stale) {
            let _ = self.consumer.pop();
        }

        let mut k = 0;
        let mut anchor: Option<(Time, usize)> = None;
        let mut publish = false;
        let mut starved = false;
        while k < frames {
            if self.read >= usize::from(self.block.frames) {
                let Ok(front) = self.consumer.peek() else {
                    starved = true;
                    break;
                };
                if stale(front) {
                    let _ = self.consumer.pop();
                    continue;
                }
                if !started && !front.grain {
                    break; // preroll: keep the ring for the start
                }
                let Ok(block) = self.consumer.pop() else {
                    starved = true;
                    break;
                };
                self.block = block;
                self.read = 0;
                if !self.block.grain {
                    if self.begin_run() {
                        anchor = None;
                    }
                    if self.block.frames == 0 {
                        // The end marker: no audio, the run ends where it is.
                        self.mark_start(k, latency_nanos, now);
                        anchor = Some((self.block.time, k));
                        self.run.valid_until = self.block.time;
                        self.run.ended = true;
                        publish = true;
                        continue;
                    }
                }
            }
            if !started && !self.block.grain {
                break;
            }
            let n = (usize::from(self.block.frames) - self.read).min(frames - k);
            self.copy(out, k, n);
            if !self.block.grain {
                self.mark_start(k, latency_nanos, now);
                if anchor.is_none() {
                    anchor = Some((self.block.time_at(self.read, self.sample_rate), k));
                }
                self.run.valid_until = self.block.time_at(self.read + n, self.sample_rate);
                if self.read + n == usize::from(self.block.frames) && self.block.last {
                    self.run.ended = true;
                }
                publish = true;
            }
            self.read += n;
            k += n;
        }
        for sample in &mut out[k * channels..] {
            *sample = T::SILENCE;
        }

        if starved && started && self.run.active && self.run.session == session && !self.run.ended {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            self.run.resume = true;
        }
        if publish && let Some((time, at)) = anchor {
            let run = &self.run;
            self.clock.publish(&ClockSample {
                session: run.session,
                seek_gen: run.seek_gen,
                epoch: run.epoch,
                time,
                at_dac: self.dac_time(at, latency_nanos, now),
                valid_until: run.valid_until,
                rate: run.rate,
                run_start: run.start,
                run_at: run.start_at,
                run_kind: run.kind,
                ended: run.ended,
            });
        }
    }

    /// When frame `k` of this buffer reaches the DAC.
    fn dac_time(&self, k: usize, latency_nanos: u64, now: u64) -> u64 {
        let offset = (k as u128) * NANOS_PER_SECOND / u128::from(self.sample_rate);
        now.saturating_add(latency_nanos)
            .saturating_add(u64::try_from(offset).unwrap_or(u64::MAX))
    }

    fn mark_start(&mut self, k: usize, latency_nanos: u64, now: u64) {
        if self.run.start_pending {
            self.run.start_at = self.dac_time(k, latency_nanos, now);
            self.run.start_pending = false;
        }
    }

    /// Starts a new run if the newly popped (non-grain) block begins one; true if it did.
    fn begin_run(&mut self) -> bool {
        let b = &self.block;
        let run = &self.run;
        let kind = if !run.active || run.session != b.session {
            RunStart::First
        } else if run.epoch != b.epoch {
            RunStart::Follows {
                previous_end: run.valid_until,
                previous_rate: run.rate,
            }
        } else if run.resume {
            RunStart::Resumed
        } else {
            return false;
        };
        self.run = Run {
            active: true,
            session: b.session,
            seek_gen: b.seek_gen,
            epoch: b.epoch,
            rate: b.rate,
            start: b.time,
            start_at: 0,
            start_pending: true,
            kind,
            valid_until: b.time,
            ended: false,
            resume: false,
        };
        true
    }

    /// Copies `n` frames of the current block into `out` from frame `k`, converting stereo to
    /// the device's channels: mono averages, more than two get L, R, then silence.
    fn copy<T: OutputSample>(&self, out: &mut [T], k: usize, n: usize) {
        let channels = self.channels;
        let source = &self.block.samples[2 * self.read..2 * (self.read + n)];
        let target = &mut out[k * channels..(k + n) * channels];
        match channels {
            1 => {
                for (o, s) in target.iter_mut().zip(source.as_chunks::<2>().0) {
                    *o = T::from_f32((s[0] + s[1]) * 0.5);
                }
            }
            2 => {
                for (o, s) in target.iter_mut().zip(source) {
                    *o = T::from_f32(*s);
                }
            }
            _ => {
                for (frame, s) in target
                    .chunks_exact_mut(channels)
                    .zip(source.as_chunks::<2>().0)
                {
                    frame[0] = T::from_f32(s[0]);
                    frame[1] = T::from_f32(s[1]);
                    for o in &mut frame[2..] {
                        *o = T::SILENCE;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::ring::{BLOCK_FRAMES, ring};
    use crate::playback::clock::{AudioClock, ClockReading};

    struct Rig {
        producer: super::super::ring::BlockProducer,
        callback: OutputCallback,
        transport: Arc<TransportShared>,
        cell: Arc<ClockCell>,
        clock: AudioClock,
        shared: Arc<AudioShared>,
        session: u32,
    }

    fn rig(channels: usize) -> Rig {
        let (producer, consumer) = ring();
        let transport = Arc::new(TransportShared::new());
        let cell = Arc::new(ClockCell::new());
        let shared = Arc::new(AudioShared::new());
        let session = cell.begin_session();
        let callback = OutputCallback::new(
            consumer,
            Arc::clone(&transport),
            Arc::clone(&cell),
            Arc::clone(&shared),
            channels,
            48_000,
        );
        let clock = AudioClock::new(Arc::clone(&cell), Arc::clone(&transport));
        Rig {
            producer,
            callback,
            transport,
            cell,
            clock,
            shared,
            session,
        }
    }

    fn block(session: u32, epoch: u32, index: i64, value: f32) -> AudioBlock {
        let time = crate::audio::sample_offset(index * BLOCK_FRAMES as i64, 48_000);
        let mut samples = [0.0; 2 * BLOCK_FRAMES];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = if i % 2 == 0 { value } else { -value };
        }
        AudioBlock {
            session,
            epoch,
            time,
            end: time + crate::audio::sample_offset(BLOCK_FRAMES as i64, 48_000),
            frames: BLOCK_FRAMES as u16,
            samples,
            ..AudioBlock::EMPTY
        }
    }

    const MS: u64 = 1_000_000;

    #[test]
    fn silence_until_started_then_blocks_and_the_clock() {
        let mut r = rig(2);
        for i in 0..4 {
            r.producer.push(block(r.session, 1, i, 0.5)).unwrap();
        }
        let mut out = [1.0f32; 2 * 480];
        r.callback.process_at(&mut out, 20 * MS, 100 * MS);
        assert!(out.iter().all(|&s| s == 0.0), "preroll: silence");
        assert_eq!(r.producer.slots(), 64 - 4, "the ring is kept for the start");
        assert_eq!(r.clock.reading_at(200 * MS), None);

        r.transport.set_started(true);
        r.callback.process_at(&mut out, 20 * MS, 110 * MS);
        assert_eq!(&out[..2], &[0.5, -0.5]);
        assert_eq!(&out[2 * 479..], &[0.5, -0.5]);
        assert_eq!(r.clock.reading_at(129 * MS), None, "not audible yet");
        let at_dac = r.clock.reading_at(130 * MS).unwrap();
        assert_eq!(
            at_dac.time,
            Time::ZERO,
            "the clock starts with the first sample"
        );
        let later = r.clock.reading_at(135 * MS).unwrap();
        assert_eq!(later.time, crate::audio::sample_offset(240, 48_000));
        assert_eq!(r.shared.underruns(), 0);
    }

    #[test]
    fn stale_generations_are_dropped_and_underruns_freeze_the_clock() {
        let mut r = rig(2);
        r.transport.set_started(true);
        r.producer.push(block(r.session, 1, 0, 0.25)).unwrap();
        let generation = r.transport.seek(Time::ZERO);
        let mut fresh = block(r.session, 1, 0, 0.75);
        fresh.seek_gen = generation;
        r.producer.push(fresh).unwrap();
        let mut out = [0.0f32; 2 * 300];
        r.callback.process_at(&mut out, 0, 0);
        assert_eq!(out[0], 0.75, "the old generation's block was dropped");
        assert_eq!(out[2 * 255], 0.75);
        assert_eq!(out[2 * 256], 0.0, "underrun: silence");
        assert_eq!(r.shared.underruns(), 1);
        let frozen = r.clock.reading_at(100 * MS).unwrap();
        assert_eq!(frozen.time, crate::audio::sample_offset(256, 48_000));
        assert!(!frozen.ended);
    }

    #[test]
    fn loop_wraps_publish_the_next_epoch() {
        let mut r = rig(2);
        r.transport.set_started(true);
        r.producer.push(block(r.session, 1, 0, 0.1)).unwrap();
        r.producer.push(block(r.session, 2, 0, 0.2)).unwrap();
        let mut out = [0.0f32; 2 * 512];
        r.callback.process_at(&mut out, 0, 0);
        let five_ms = crate::audio::sample_offset(256, 48_000);
        let r1 = r.clock.reading_at(1).unwrap();
        assert_eq!(r1.epoch, 1, "the old run plays until the wrap is heard");
        let ms_256 = (256 * 1_000_000_000u64) / 48_000;
        let r2: ClockReading = r.clock.reading_at(ms_256 + MS).unwrap();
        assert_eq!(r2.epoch, 2);
        assert!(r2.time < five_ms);
    }

    #[test]
    fn channels_are_converted() {
        let mut mono = rig(1);
        mono.transport.set_started(true);
        let mut b = block(mono.session, 1, 0, 0.0);
        for (i, s) in b.samples.iter_mut().enumerate() {
            *s = if i % 2 == 0 { 0.5 } else { 0.25 };
        }
        mono.producer.push(b).unwrap();
        let mut out = [0.0f32; 10];
        mono.callback.process_at(&mut out, 0, 0);
        assert!(out.iter().all(|&s| s == 0.375));

        let mut six = rig(6);
        six.transport.set_started(true);
        six.producer.push(block(six.session, 1, 0, 0.5)).unwrap();
        let mut out = [9i16; 12];
        six.callback.process_at(&mut out, 0, 0);
        assert_eq!(&out[..6], &[16_384, -16_384, 0, 0, 0, 0]);
        assert_eq!(&out[6..], &[16_384, -16_384, 0, 0, 0, 0]);
    }

    #[test]
    fn sample_formats_convert_and_clamp() {
        assert_eq!(i16::from_f32(1.5), 32_767);
        assert_eq!(i16::from_f32(-1.0), -32_767);
        assert_eq!(i32::from_f32(1.0), i32::MAX);
        assert_eq!(u16::from_f32(0.0), 32_768);
        assert_eq!(u16::from_f32(-1.0), 1);
        assert_eq!(u16::from_f32(1.0), 65_535);
    }

    #[test]
    fn grains_play_while_paused_without_moving_the_clock() {
        let mut r = rig(2);
        let mut grain = block(r.session, 0, 10, 0.3);
        grain.grain = true;
        r.producer.push(grain).unwrap();
        let mut out = [0.0f32; 2 * 256];
        r.callback.process_at(&mut out, 0, 0);
        assert_eq!(out[0], 0.3);
        assert_eq!(r.clock.reading_at(MS), None);
        assert!(r.cell.load().is_none());
        assert_eq!(r.shared.underruns(), 0);
    }

    #[test]
    fn the_end_is_published() {
        let mut r = rig(2);
        r.transport.set_started(true);
        let mut last = block(r.session, 1, 0, 0.1);
        last.frames = 100;
        last.end = crate::audio::sample_offset(100, 48_000);
        last.last = true;
        r.producer.push(last).unwrap();
        let mut out = [0.0f32; 2 * 256];
        r.callback.process_at(&mut out, 0, 0);
        assert_eq!(r.shared.underruns(), 0, "the end is no underrun");
        let end = r.clock.reading_at(100 * MS).unwrap();
        assert!(end.ended);
        assert_eq!(end.time, last.end);
    }
}
