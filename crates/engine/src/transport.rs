//! Transport state shared between the IPC thread and the engine threads (playback.md §4.1).
//!
//! Task 0320 has only the paused playhead and the seek generation: the UI playhead drives the
//! preview through `seek`. Task 0330 adds play state, rate, loop and skim to [`TransportShared`].

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use app_core::Time;

/// How a seek is meant (playback.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    /// A click, a step or the end of a drag: show the exact frame at the new time.
    Exact,
    /// The playhead is being dragged: show the nearest ready frame at once, refine later.
    Scrub,
}

/// Transport values read by the render thread (and later the audio feeder and callback) without
/// locks. Single writer: the main thread, through [`crate::preview::PreviewHandle::seek`].
#[derive(Debug, Default)]
pub struct TransportShared {
    /// Playhead while paused, in flicks. During playback (0330) the clock is the truth.
    playhead: AtomicI64,
    /// Incremented by every seek. The UI ignores ticks of an older generation (0330).
    seek_gen: AtomicU32,
}

impl TransportShared {
    pub fn new() -> Self {
        Self::default()
    }

    /// The paused playhead.
    pub fn playhead(&self) -> Time {
        Time(self.playhead.load(Ordering::Acquire))
    }

    /// The generation of the newest seek; 0 before the first one.
    pub fn seek_generation(&self) -> u32 {
        self.seek_gen.load(Ordering::Acquire)
    }

    /// Moves the playhead to `time` (already clamped by the caller) and returns the new seek
    /// generation. Generations wrap at `u32::MAX`, which takes years of scrubbing.
    pub fn seek(&self, time: Time) -> u32 {
        self.playhead.store(time.flicks(), Ordering::Release);
        self.seek_gen.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }
}

/// `time` clamped to the timeline, `0..=duration` (playback.md §4.2).
pub fn clamp_to_timeline(time: Time, duration: Time) -> Time {
    time.clamp(Time::ZERO, duration.max(Time::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeks_move_the_playhead_and_increase_the_generation() {
        let transport = TransportShared::new();
        assert_eq!(transport.playhead(), Time::ZERO);
        assert_eq!(transport.seek_generation(), 0);

        assert_eq!(transport.seek(Time(100)), 1);
        assert_eq!(transport.playhead(), Time(100));
        assert_eq!(transport.seek(Time(50)), 2);
        assert_eq!(transport.playhead(), Time(50));
        assert_eq!(transport.seek_generation(), 2);
    }

    #[test]
    fn clamps_to_the_timeline() {
        let duration = Time::from_seconds(10);
        assert_eq!(clamp_to_timeline(Time(-1), duration), Time::ZERO);
        assert_eq!(clamp_to_timeline(Time(5), duration), Time(5));
        assert_eq!(
            clamp_to_timeline(Time::from_seconds(11), duration),
            duration
        );
        // An empty timeline has only time 0.
        assert_eq!(clamp_to_timeline(Time(5), Time::ZERO), Time::ZERO);
    }
}
