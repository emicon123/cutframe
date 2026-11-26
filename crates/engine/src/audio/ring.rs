//! The ring between the audio feeder and the audio callback (playback.md §6.2).
//!
//! One `rtrb` ring of fixed-size, `Copy` [`AudioBlock`]s: the ring preallocates every slot once,
//! `push` and `pop` are wait-free, so the callback never allocates or locks.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use app_core::Time;

use super::sample_offset;

/// Frames per block: 5.3 ms at 48 kHz.
pub const BLOCK_FRAMES: usize = 256;
/// Ring capacity in blocks: about 340 ms at 48 kHz.
pub const RING_BLOCKS: usize = 64;
/// The feeder keeps the ring this full: about 85 ms at 48 kHz. An edit is heard within about
/// 120 ms.
pub const TARGET_FILL_BLOCKS: usize = 16;

/// Mixed audio for `frames` device frames, stereo `f32` interleaved, tagged with where it
/// belongs (playback.md §6.2).
#[derive(Clone, Copy)]
pub struct AudioBlock {
    /// The audio engine's play session ([`crate::playback::clock::ClockCell::begin_session`]).
    pub session: u32,
    pub seek_gen: u32,
    /// See [`crate::playback::clock::ClockSample::epoch`]. Unused for grains.
    pub epoch: u32,
    /// Timeline time of the first frame.
    pub time: Time,
    /// Timeline time just after the last frame (`time + frames x rate` samples, clamped to the
    /// end of the timeline for the last block before it).
    pub end: Time,
    /// `<= BLOCK_FRAMES`. 0 only for the end marker: `last` with no audio, when playback reaches
    /// the end exactly at a block start.
    pub frames: u16,
    /// Playback rate: block time runs `rate` times as fast as the device's samples. Silent
    /// unless 1.
    pub rate: i8,
    /// A scrub snippet (§6.5): played while paused, never moves the clock.
    pub grain: bool,
    /// The last block before the end of the timeline (or 0 in reverse) without loop.
    pub last: bool,
    pub samples: [f32; 2 * BLOCK_FRAMES],
}

impl std::fmt::Debug for AudioBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioBlock")
            .field("session", &self.session)
            .field("seek_gen", &self.seek_gen)
            .field("epoch", &self.epoch)
            .field("time", &self.time)
            .field("end", &self.end)
            .field("frames", &self.frames)
            .field("rate", &self.rate)
            .field("grain", &self.grain)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl AudioBlock {
    /// A silent block of no frames.
    pub const EMPTY: AudioBlock = AudioBlock {
        session: 0,
        seek_gen: 0,
        epoch: 0,
        time: Time::ZERO,
        end: Time::ZERO,
        frames: 0,
        rate: 1,
        grain: false,
        last: false,
        samples: [0.0; 2 * BLOCK_FRAMES],
    };

    /// Timeline time of frame `k` (`k <= frames`; `frames` gives [`Self::end`]).
    pub fn time_at(&self, k: usize, sample_rate: u32) -> Time {
        if k >= usize::from(self.frames) {
            return self.end;
        }
        let samples = i64::try_from(k).unwrap_or(i64::MAX) * i64::from(self.rate);
        self.time + sample_offset(samples, sample_rate)
    }
}

/// The feeder's end of the ring.
pub type BlockProducer = rtrb::Producer<AudioBlock>;
/// The callback's end of the ring.
pub type BlockConsumer = rtrb::Consumer<AudioBlock>;

/// A new ring of [`RING_BLOCKS`] blocks (allocates; call it outside real-time code).
pub fn ring() -> (BlockProducer, BlockConsumer) {
    rtrb::RingBuffer::new(RING_BLOCKS)
}

/// Counters shared by the feeder, the callback and the engine handle. Atomics only, so the
/// callback may write them.
#[derive(Debug, Default)]
pub struct AudioShared {
    /// Device buffers that found the ring empty during playback.
    pub(crate) underruns: AtomicU64,
    /// The session whose ring reached its preroll target (or its end).
    pub(crate) ready_session: AtomicU32,
    /// Blocks in the ring, as the feeder last saw it, and the lowest value since the last
    /// [`Self::take_min_fill`].
    pub(crate) fill: AtomicU32,
    pub(crate) min_fill: AtomicU32,
    /// Callback timing for playback.md §11: total time spent in the callback, total device time
    /// those buffers covered, the worst single buffer in parts per million of its period, and
    /// the number of calls.
    pub(crate) busy_nanos: AtomicU64,
    pub(crate) period_nanos: AtomicU64,
    pub(crate) max_load_ppm: AtomicU32,
    pub(crate) calls: AtomicU64,
    /// The device's reported latency (playback minus callback timestamp) of the newest buffer.
    pub(crate) latency_nanos: AtomicU64,
}

impl AudioShared {
    pub fn new() -> Self {
        Self {
            min_fill: AtomicU32::new(u32::MAX),
            ..Self::default()
        }
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// The lowest ring fill since the previous call (`None` if nothing was measured).
    pub fn take_min_fill(&self) -> Option<u32> {
        match self.min_fill.swap(u32::MAX, Ordering::Relaxed) {
            u32::MAX => None,
            fill => Some(fill),
        }
    }

    pub(crate) fn record_fill(&self, fill: u32) {
        self.fill.store(fill, Ordering::Relaxed);
        self.min_fill.fetch_min(fill, Ordering::Relaxed);
    }

    pub(crate) fn record_call(&self, busy: u64, period: u64) {
        self.busy_nanos.fetch_add(busy, Ordering::Relaxed);
        self.period_nanos.fetch_add(period, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
        if period > 0 {
            let ppm = u32::try_from(u128::from(busy) * 1_000_000 / u128::from(period))
                .unwrap_or(u32::MAX);
            self.max_load_ppm.fetch_max(ppm, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_times_follow_the_rate_and_end_at_end() {
        let block = AudioBlock {
            time: Time(1_000_000),
            end: Time(1_000_000 - 2 * 256 * 14_700),
            frames: 256,
            rate: -2,
            ..AudioBlock::EMPTY
        };
        assert_eq!(block.time_at(0, 48_000), Time(1_000_000));
        assert_eq!(block.time_at(1, 48_000), Time(1_000_000 - 2 * 14_700));
        assert_eq!(block.time_at(256, 48_000), block.end);
    }

    #[test]
    fn the_ring_holds_its_capacity() {
        let (mut producer, mut consumer) = ring();
        for i in 0..RING_BLOCKS {
            let block = AudioBlock {
                seek_gen: i as u32,
                ..AudioBlock::EMPTY
            };
            producer.push(block).unwrap();
        }
        assert!(producer.push(AudioBlock::EMPTY).is_err());
        assert_eq!(consumer.pop().unwrap().seek_gen, 0);
    }
}
