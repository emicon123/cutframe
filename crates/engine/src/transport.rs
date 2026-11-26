//! Transport state shared between the IPC thread and the engine threads (playback.md §4.1).
//!
//! The atomics every playback thread reads: the main thread writes them (through the transport
//! commands), the render thread, the audio feeder and the real-time audio callback read them
//! without locks. The state machine that decides the writes lives in `crate::playback` (0330).

use std::sync::atomic::{AtomicBool, AtomicI8, AtomicI64, AtomicU8, AtomicU32, Ordering};

use app_core::Time;

/// How a seek is meant (playback.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    /// A click, a step or the end of a drag: show the exact frame at the new time.
    Exact,
    /// The playhead is being dragged: show the nearest ready frame at once, refine later.
    Scrub,
}

/// Play state as stored in [`TransportShared`] (playback.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Paused,
    /// Play was requested: readers prime the first frame and the feeder fills the ring. Shown to
    /// the UI as playing.
    Prerolling,
    Playing,
}

impl PlayState {
    fn to_u8(self) -> u8 {
        match self {
            Self::Paused => 0,
            Self::Prerolling => 1,
            Self::Playing => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Prerolling,
            2 => Self::Playing,
            _ => Self::Paused,
        }
    }
}

/// Transport values shared by the main thread (single writer) and the render thread, the audio
/// feeder and the audio callback (readers), without locks (playback.md §4.1). Every accessor is a
/// single atomic load or store, so the real-time callback may call the readers.
#[derive(Debug)]
pub struct TransportShared {
    /// Playhead while paused, in flicks. During playback the clock is the truth.
    playhead: AtomicI64,
    /// Incremented by every seek, step, target change and project replacement. The UI ignores
    /// ticks of an older generation; the audio callback drops blocks of an older one.
    seek_gen: AtomicU32,
    /// A [`PlayState`].
    state: AtomicU8,
    /// Shuttle rate: one of -8, -4, -2, -1, 1, 2, 4, 8 while not paused; 0 while paused.
    rate: AtomicI8,
    looping: AtomicBool,
    /// Preroll finished: the audio callback may consume the ring.
    started: AtomicBool,
    /// Skim time (0340), `i64::MIN` for none. Honoured only while paused.
    skim: AtomicI64,
    /// Start of the project frame on screen while playing (written by the render thread, and by
    /// the transport when playback starts). Pause snaps the playhead to it; ticks report it.
    shown: AtomicI64,
}

impl Default for TransportShared {
    fn default() -> Self {
        Self {
            playhead: AtomicI64::new(0),
            seek_gen: AtomicU32::new(0),
            state: AtomicU8::new(PlayState::Paused.to_u8()),
            rate: AtomicI8::new(0),
            looping: AtomicBool::new(false),
            started: AtomicBool::new(false),
            skim: AtomicI64::new(i64::MIN),
            shown: AtomicI64::new(0),
        }
    }
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

    /// Sets the paused playhead without a new generation (pause snapping to the shown frame).
    pub fn set_playhead(&self, time: Time) {
        self.playhead.store(time.flicks(), Ordering::Release);
    }

    pub fn state(&self) -> PlayState {
        PlayState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn set_state(&self, state: PlayState) {
        self.state.store(state.to_u8(), Ordering::Release);
    }

    /// Prerolling or playing: what the UI shows as playing, and what the export throttle reads.
    pub fn is_playing(&self) -> bool {
        self.state() != PlayState::Paused
    }

    /// The shuttle rate; 0 while paused.
    pub fn rate(&self) -> i8 {
        self.rate.load(Ordering::Acquire)
    }

    pub fn set_rate(&self, rate: i8) {
        self.rate.store(rate, Ordering::Release);
    }

    pub fn looping(&self) -> bool {
        self.looping.load(Ordering::Acquire)
    }

    pub fn set_looping(&self, on: bool) {
        self.looping.store(on, Ordering::Release);
    }

    /// Preroll finished: the audio callback may consume the ring.
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub fn set_started(&self, started: bool) {
        self.started.store(started, Ordering::Release);
    }

    /// The skim time (0340), if any.
    pub fn skim(&self) -> Option<Time> {
        match self.skim.load(Ordering::Acquire) {
            i64::MIN => None,
            flicks => Some(Time(flicks)),
        }
    }

    pub fn set_skim(&self, time: Option<Time>) {
        let flicks = time.map_or(i64::MIN, Time::flicks);
        self.skim.store(flicks, Ordering::Release);
    }

    /// Start of the project frame on screen while playing.
    pub fn shown(&self) -> Time {
        Time(self.shown.load(Ordering::Acquire))
    }

    pub fn set_shown(&self, time: Time) {
        self.shown.store(time.flicks(), Ordering::Release);
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
    fn play_state_rate_loop_and_skim_round_trip() {
        let transport = TransportShared::new();
        assert_eq!(transport.state(), PlayState::Paused);
        assert!(!transport.is_playing());
        assert_eq!(transport.rate(), 0);
        assert_eq!(transport.skim(), None);
        for state in [PlayState::Prerolling, PlayState::Playing, PlayState::Paused] {
            transport.set_state(state);
            assert_eq!(transport.state(), state);
        }
        transport.set_state(PlayState::Prerolling);
        assert!(transport.is_playing());
        transport.set_rate(-4);
        assert_eq!(transport.rate(), -4);
        transport.set_looping(true);
        assert!(transport.looping());
        transport.set_started(true);
        assert!(transport.started());
        transport.set_skim(Some(Time(7)));
        assert_eq!(transport.skim(), Some(Time(7)));
        transport.set_skim(None);
        assert_eq!(transport.skim(), None);
        transport.set_playhead(Time(42));
        assert_eq!(transport.playhead(), Time(42));
        assert_eq!(transport.seek_generation(), 0, "set_playhead is not a seek");
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
