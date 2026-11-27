//! The transport state machine (playback.md §4): Paused, Prerolling and Playing, the shuttle
//! rates, and what play, pause, J/K/L, step, seek, loop, end and project replacement do.
//!
//! [`Transport`] is pure logic over [`TransportShared`]: each input writes the atomics the engine
//! threads read and returns the side effects ([`Effects`]) for the caller to carry out (wake the
//! render thread, tell the audio path). It never blocks and never allocates beyond the returned
//! list. The caller serialises inputs (`crate::playback::Playback` holds it under a mutex shared
//! by the main thread and the render thread).

use std::sync::Arc;

use app_core::{FrameRate, Time};

use crate::transport::{PlayState, SeekMode, TransportShared, clamp_to_timeline};

/// Shuttle rate: one of -8, -4, -2, -1, 1, 2, 4, 8 (playback.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rate(i8);

impl Rate {
    pub const FORWARD: Self = Self(1);
    pub const REVERSE: Self = Self(-1);
    /// The fastest shuttle speed.
    pub const MAX: i8 = 8;

    /// `rate` if it is a shuttle rate.
    pub fn new(rate: i8) -> Option<Self> {
        matches!(rate.unsigned_abs(), 1 | 2 | 4 | 8).then_some(Self(rate))
    }

    pub fn get(self) -> i8 {
        self.0
    }

    pub fn is_forward(self) -> bool {
        self.0 > 0
    }

    /// `L`: 1x from pause or reverse, else double up to 8x.
    pub fn faster_forward(current: Option<Self>) -> Self {
        match current {
            Some(Self(rate)) if rate > 0 => Self((rate * 2).min(Self::MAX)),
            _ => Self::FORWARD,
        }
    }

    /// `J`: -1x from pause or forward, else double down to -8x.
    pub fn faster_reverse(current: Option<Self>) -> Self {
        match current {
            Some(Self(rate)) if rate < 0 => Self((rate * 2).max(-Self::MAX)),
            _ => Self::REVERSE,
        }
    }
}

/// The transport state (playback.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    Paused,
    /// Play was requested: readers prime the first frame and the feeder fills the ring. Shown to
    /// the UI as playing.
    Prerolling {
        rate: Rate,
    },
    Playing {
        rate: Rate,
    },
}

impl TransportState {
    /// The rate while not paused.
    pub fn rate(self) -> Option<Rate> {
        match self {
            Self::Paused => None,
            Self::Prerolling { rate } | Self::Playing { rate } => Some(rate),
        }
    }

    pub fn is_paused(self) -> bool {
        self == Self::Paused
    }
}

/// J, K and L.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShuttleKey {
    Reverse,
    Pause,
    Forward,
}

/// What the transport needs to know about the timeline it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    pub duration: Time,
    pub frame_rate: FrameRate,
}

impl Timeline {
    pub fn of(project: &app_core::Project) -> Self {
        Self {
            duration: project.duration(),
            frame_rate: project.settings.frame_rate,
        }
    }

    /// Index of the last frame, `floor((duration - 1) / frame)`; 0 for an empty timeline.
    pub fn last_frame(self) -> i64 {
        if self.duration <= Time::ZERO {
            return 0;
        }
        (self.duration - Time(1)).to_frames(self.frame_rate)
    }

    /// Start of the last frame, where playback without loop pauses.
    pub fn last_frame_start(self) -> Time {
        Time::from_frames(self.last_frame(), self.frame_rate)
    }

    /// Start of the frame that contains `t`, clamped to `0..=last_frame_start`.
    pub fn frame_start(self, t: Time) -> Time {
        let frame = t.to_frames(self.frame_rate).clamp(0, self.last_frame());
        Time::from_frames(frame, self.frame_rate)
    }

    /// Whether play from `t` would show nothing new: on the last frame or past it.
    pub fn is_at_end(self, t: Time) -> bool {
        self.duration <= Time::ZERO || t >= self.last_frame_start()
    }
}

/// What the audio path is told (`crate::playback::AudioSink`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCommand {
    /// Prerolling at `from` with `rate`, generation `seek_gen`: fill the ring from there.
    Play {
        from: Time,
        rate: Rate,
        seek_gen: u32,
    },
    /// Same direction, another speed; playback continues without a new generation.
    SetRate(Rate),
    Pause,
    /// A seek while paused (a `Scrub` seek plays a grain, playback.md §6.5).
    Seek {
        to: Time,
        seek_gen: u32,
        mode: SeekMode,
    },
    SetLooping(bool),
}

/// The side effects of one input, for the caller to carry out after the atomics are written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effects {
    /// Something the UI sees changed: wake the render thread, which redraws and emits a tick.
    pub changed: bool,
    /// The seek generation and mode of a seek or step in this input.
    pub seek: Option<(u32, SeekMode)>,
    pub audio: Vec<AudioCommand>,
}

impl Effects {
    fn none() -> Self {
        Self::default()
    }

    fn changed(audio: Vec<AudioCommand>) -> Self {
        Self {
            changed: true,
            seek: None,
            audio,
        }
    }
}

/// The transport rules of playback.md §4.2 over the shared atomics. See the module docs.
#[derive(Debug)]
pub struct Transport {
    shared: Arc<TransportShared>,
    /// The timeline playhead while a media target plays (playback.md §8): kept, and restored
    /// when the timeline is the target again.
    timeline_playhead: Time,
}

impl Transport {
    pub fn new(shared: Arc<TransportShared>) -> Self {
        Self {
            shared,
            timeline_playhead: Time::ZERO,
        }
    }

    pub fn shared(&self) -> &Arc<TransportShared> {
        &self.shared
    }

    /// The state as stored in the atomics.
    pub fn state(&self) -> TransportState {
        let rate = Rate::new(self.shared.rate()).unwrap_or(Rate::FORWARD);
        match self.shared.state() {
            PlayState::Paused => TransportState::Paused,
            PlayState::Prerolling => TransportState::Prerolling { rate },
            PlayState::Playing => TransportState::Playing { rate },
        }
    }

    fn store(&self, state: TransportState) {
        let (play, rate) = match state {
            TransportState::Paused => (PlayState::Paused, 0),
            TransportState::Prerolling { rate } => (PlayState::Prerolling, rate.get()),
            TransportState::Playing { rate } => (PlayState::Playing, rate.get()),
        };
        // Rate first: a reader that sees the new state also sees its rate.
        self.shared.set_rate(rate);
        self.shared.set_state(play);
    }

    /// Starts prerolling at `from` (a new generation, so stale clock samples and audio blocks of
    /// the last run are ignored).
    fn start(&self, from: Time, rate: Rate) -> Effects {
        self.shared.set_started(false);
        self.shared.set_skim(None);
        let seek_gen = self.shared.seek(from);
        self.shared.set_shown(from);
        self.store(TransportState::Prerolling { rate });
        Effects::changed(vec![AudioCommand::Play {
            from,
            rate,
            seek_gen,
        }])
    }

    /// Where play starts: the playhead, or 0 when it is at the end (Space at the end).
    fn play_from(&self, timeline: Timeline) -> Option<Time> {
        if timeline.duration <= Time::ZERO {
            return None;
        }
        let playhead = self.shared.playhead();
        Some(if timeline.is_at_end(playhead) {
            Time::ZERO
        } else {
            playhead
        })
    }

    /// Space / the Play button: 1x from the playhead (0 at the end), or pause.
    pub fn toggle(&self, timeline: Timeline) -> Effects {
        if !self.state().is_paused() {
            return self.pause(timeline);
        }
        match self.play_from(timeline) {
            Some(from) => self.start(from, Rate::FORWARD),
            None => Effects::none(),
        }
    }

    /// Pauses on the start of the frame on screen, so "split at playhead" cuts what the user
    /// sees. Paused already: nothing changes.
    pub fn pause(&self, timeline: Timeline) -> Effects {
        if self.state().is_paused() {
            return Effects::none();
        }
        let at = timeline.frame_start(self.shared.shown());
        self.stop_at(at)
    }

    fn stop_at(&self, at: Time) -> Effects {
        self.shared.set_started(false);
        self.shared.set_playhead(at);
        self.shared.set_shown(at);
        self.store(TransportState::Paused);
        Effects::changed(vec![AudioCommand::Pause])
    }

    /// J, K, L (playback.md §4.2).
    pub fn shuttle(&self, key: ShuttleKey, timeline: Timeline) -> Effects {
        let state = self.state();
        let current = state.rate();
        let rate = match key {
            ShuttleKey::Pause => return self.pause(timeline),
            ShuttleKey::Forward => Rate::faster_forward(current),
            ShuttleKey::Reverse => Rate::faster_reverse(current),
        };
        match current {
            None => {
                let from = if rate.is_forward() {
                    self.play_from(timeline)
                } else {
                    // Reverse playback pauses at 0: nothing to play from there.
                    let playhead = self.shared.playhead();
                    (playhead > Time::ZERO).then_some(playhead)
                };
                match from {
                    Some(from) => self.start(from, rate),
                    None => Effects::none(),
                }
            }
            Some(current) if current == rate => Effects::none(),
            Some(current) if current.is_forward() == rate.is_forward() => {
                let next = match state {
                    TransportState::Prerolling { .. } => TransportState::Prerolling { rate },
                    _ => TransportState::Playing { rate },
                };
                self.store(next);
                Effects::changed(vec![AudioCommand::SetRate(rate)])
            }
            // A direction change restarts from the frame on screen (new generation, preroll).
            Some(_) => self.start(timeline.frame_start(self.shared.shown()), rate),
        }
    }

    /// Left / Right: pauses, then moves `frames` whole frames from the current frame, clamped
    /// to the first and last frame. Returns the new seek generation.
    pub fn step(&self, frames: i32, timeline: Timeline) -> (u32, Effects) {
        let mut effects = self.pause(timeline);
        let current = self.shared.playhead().to_frames(timeline.frame_rate);
        let target = current
            .saturating_add(i64::from(frames))
            .clamp(0, timeline.last_frame());
        let to = Time::from_frames(target, timeline.frame_rate);
        let seek_gen = self.shared.seek(to);
        self.shared.set_shown(to);
        effects.changed = true;
        effects.seek = Some((seek_gen, SeekMode::Exact));
        effects.audio.push(AudioCommand::Seek {
            to,
            seek_gen,
            mode: SeekMode::Exact,
        });
        (seek_gen, effects)
    }

    /// Seek (playback.md §4.2): clamps to `0..=duration`, a new generation. `Exact` while
    /// playing prerolls again at the new time; `Scrub` pauses (and plays a grain). Returns the
    /// new seek generation.
    pub fn seek(&self, time: Time, mode: SeekMode, timeline: Timeline) -> (u32, Effects) {
        let to = clamp_to_timeline(time, timeline.duration);
        let mut effects = match (self.state().rate(), mode) {
            (Some(rate), SeekMode::Exact) => self.start(to, rate),
            (Some(_), SeekMode::Scrub) => self.stop_at(to),
            (None, _) => Effects::changed(Vec::new()),
        };
        let seek_gen = if effects
            .audio
            .iter()
            .any(|a| matches!(a, AudioCommand::Play { .. }))
        {
            // `start` already made the generation.
            self.shared.seek_generation()
        } else {
            let seek_gen = self.shared.seek(to);
            self.shared.set_shown(to);
            effects
                .audio
                .push(AudioCommand::Seek { to, seek_gen, mode });
            seek_gen
        };
        effects.seek = Some((seek_gen, mode));
        (seek_gen, effects)
    }

    /// The loop button: while on, playback wraps from the end to 0.
    pub fn set_loop(&self, on: bool) -> Effects {
        self.shared.set_looping(on);
        Effects::changed(vec![AudioCommand::SetLooping(on)])
    }

    /// The preview's target changes (playback.md §8, task 0340): pauses on the frame on screen
    /// of `before` (the range of the old target), keeps the playhead when `leaving_timeline`,
    /// then lets `switch` change the target and return the new range, and moves to the kept
    /// timeline playhead when `entering_timeline`, else to 0 (a new generation, the skim
    /// cleared).
    pub fn retarget(
        &mut self,
        before: Timeline,
        leaving_timeline: bool,
        entering_timeline: bool,
        switch: impl FnOnce() -> Timeline,
    ) -> Effects {
        let mut effects = self.pause(before);
        if leaving_timeline {
            self.timeline_playhead = self.shared.playhead();
        }
        let after = switch();
        let to = if entering_timeline {
            self.timeline_playhead
        } else {
            Time::ZERO
        };
        let to = clamp_to_timeline(to, after.duration);
        self.shared.set_skim(None);
        let seek_gen = self.shared.seek(to);
        self.shared.set_shown(to);
        effects.changed = true;
        effects.seek = Some((seek_gen, SeekMode::Exact));
        effects.audio.push(AudioCommand::Seek {
            to,
            seek_gen,
            mode: SeekMode::Exact,
        });
        effects
    }

    /// Skim (playback.md §4.2, task 0340): the time the paused preview shows instead of the
    /// playhead, clamped to `0..=duration`; `None` shows the playhead again. Ignored while
    /// playing (playing clears it). Changes no generation and no tick: returns whether the
    /// picture's input changed.
    pub fn set_skim(&self, time: Option<Time>, timeline: Timeline) -> bool {
        if !self.state().is_paused() {
            return false;
        }
        let time = time.map(|t| clamp_to_timeline(t, timeline.duration));
        if self.shared.skim() == time {
            return false;
        }
        self.shared.set_skim(time);
        true
    }

    /// A new, opened or restored project: pause, playhead 0, a new generation.
    pub fn reset(&mut self) -> Effects {
        self.timeline_playhead = Time::ZERO;
        let mut effects = if self.state().is_paused() {
            Effects::changed(Vec::new())
        } else {
            self.stop_at(Time::ZERO)
        };
        self.shared.set_skim(None);
        let seek_gen = self.shared.seek(Time::ZERO);
        self.shared.set_shown(Time::ZERO);
        effects.seek = Some((seek_gen, SeekMode::Exact));
        effects.audio.push(AudioCommand::Seek {
            to: Time::ZERO,
            seek_gen,
            mode: SeekMode::Exact,
        });
        effects
    }

    /// Render thread: preroll of generation `seek_gen` finished (first frame and ring ready, or
    /// the timeout). From now the audio callback consumes the ring. Ignored when a newer input
    /// replaced that preroll.
    pub fn preroll_done(&self, seek_gen: u32) -> Effects {
        match self.state() {
            TransportState::Prerolling { rate } if self.shared.seek_generation() == seek_gen => {
                self.store(TransportState::Playing { rate });
                self.shared.set_started(true);
                Effects::changed(Vec::new())
            }
            _ => Effects::none(),
        }
    }

    /// Render thread: playback of generation `seek_gen` reached the end (forward without loop:
    /// pause on the last frame) or the start (reverse: pause at 0). Ignored when a newer input
    /// replaced that run.
    pub fn end(&self, seek_gen: u32, timeline: Timeline) -> Effects {
        let Some(rate) = self.state().rate() else {
            return Effects::none();
        };
        if self.shared.seek_generation() != seek_gen {
            return Effects::none();
        }
        let at = if rate.is_forward() {
            timeline.last_frame_start()
        } else {
            Time::ZERO
        };
        self.stop_at(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: i64 = app_core::FLICKS_PER_SECOND / 30;

    fn timeline(seconds: i64) -> Timeline {
        Timeline {
            duration: Time::from_seconds(seconds),
            frame_rate: FrameRate::FPS_30,
        }
    }

    fn transport() -> Transport {
        Transport::new(Arc::new(TransportShared::new()))
    }

    fn rate(transport: &Transport) -> i8 {
        transport.shared.rate()
    }

    /// Simulates the render thread showing `t` while playing.
    fn show(transport: &Transport, t: Time) {
        transport.shared.set_shown(t);
    }

    #[test]
    fn rates_are_the_shuttle_steps() {
        for r in [-8, -4, -2, -1, 1, 2, 4, 8] {
            assert_eq!(Rate::new(r).map(Rate::get), Some(r));
        }
        for r in [0, 3, -3, 16, i8::MIN] {
            assert_eq!(Rate::new(r), None, "{r}");
        }
    }

    #[test]
    fn l_and_j_sequences_give_the_documented_rates() {
        let tl = timeline(60);
        let t = transport();
        t.shared.seek(Time::from_seconds(10));
        let mut seen = Vec::new();
        for _ in 0..5 {
            t.shuttle(ShuttleKey::Forward, tl);
            seen.push(rate(&t));
        }
        assert_eq!(seen, [1, 2, 4, 8, 8]);
        assert_eq!(t.shared.state(), PlayState::Prerolling, "still prerolling");

        // J from forward: -1x, then faster reverse.
        let mut seen = Vec::new();
        for _ in 0..5 {
            t.shuttle(ShuttleKey::Reverse, tl);
            seen.push(rate(&t));
        }
        assert_eq!(seen, [-1, -2, -4, -8, -8]);
        // L from reverse: 1x.
        t.shuttle(ShuttleKey::Forward, tl);
        assert_eq!(rate(&t), 1);
        // K pauses.
        let effects = t.shuttle(ShuttleKey::Pause, tl);
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(rate(&t), 0);
        assert_eq!(effects.audio, [AudioCommand::Pause]);
        // K while paused does nothing.
        assert_eq!(t.shuttle(ShuttleKey::Pause, tl), Effects::none());
    }

    #[test]
    fn same_direction_speed_changes_keep_the_generation_direction_changes_restart() {
        let tl = timeline(60);
        let t = transport();
        t.shared.seek(Time::from_seconds(10));
        t.shuttle(ShuttleKey::Forward, tl);
        let generation = t.shared.seek_generation();
        t.preroll_done(generation);
        assert_eq!(t.state(), TransportState::Playing { rate: Rate(1) });

        let effects = t.shuttle(ShuttleKey::Forward, tl);
        assert_eq!(effects.audio, [AudioCommand::SetRate(Rate(2))]);
        assert_eq!(t.state(), TransportState::Playing { rate: Rate(2) });
        assert_eq!(t.shared.seek_generation(), generation, "no new generation");

        show(&t, Time::from_seconds(12) + Time(FRAME / 2));
        let effects = t.shuttle(ShuttleKey::Reverse, tl);
        assert_eq!(t.state(), TransportState::Prerolling { rate: Rate(-1) });
        assert_eq!(t.shared.seek_generation(), generation + 1);
        assert_eq!(
            t.shared.playhead(),
            Time::from_seconds(12),
            "the frame on screen"
        );
        assert_eq!(
            effects.audio,
            [AudioCommand::Play {
                from: Time::from_seconds(12),
                rate: Rate(-1),
                seek_gen: generation + 1
            }]
        );
    }

    #[test]
    fn space_toggles_and_starts_at_zero_at_the_end() {
        let tl = timeline(10);
        let t = transport();
        t.shared.seek(Time::from_seconds(4));
        let effects = t.toggle(tl);
        assert!(effects.changed);
        assert_eq!(t.state(), TransportState::Prerolling { rate: Rate(1) });
        assert_eq!(t.shared.playhead(), Time::from_seconds(4));
        assert!(!t.shared.started());

        show(&t, Time::from_seconds(5));
        t.toggle(tl);
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(t.shared.playhead(), Time::from_seconds(5));

        // On the last frame (where playback without loop stops) and at the end itself.
        for end in [tl.last_frame_start(), tl.duration] {
            t.shared.seek(end);
            let effects = t.toggle(tl);
            assert_eq!(t.shared.playhead(), Time::ZERO, "{end:?}");
            assert_eq!(
                effects.audio,
                [AudioCommand::Play {
                    from: Time::ZERO,
                    rate: Rate(1),
                    seek_gen: t.shared.seek_generation()
                }]
            );
            t.pause(tl);
        }
        // One frame before the last one plays from there.
        t.shared.seek(tl.last_frame_start() - Time(FRAME));
        t.toggle(tl);
        assert_eq!(t.shared.playhead(), tl.last_frame_start() - Time(FRAME));
    }

    #[test]
    fn an_empty_timeline_does_not_play() {
        let tl = timeline(0);
        let t = transport();
        assert_eq!(t.toggle(tl), Effects::none());
        assert_eq!(t.shuttle(ShuttleKey::Forward, tl), Effects::none());
        assert_eq!(t.shuttle(ShuttleKey::Reverse, tl), Effects::none());
        assert_eq!(t.state(), TransportState::Paused);
    }

    #[test]
    fn reverse_from_zero_does_nothing() {
        let tl = timeline(10);
        let t = transport();
        assert_eq!(t.shuttle(ShuttleKey::Reverse, tl), Effects::none());
        t.shared.seek(Time(FRAME));
        t.shuttle(ShuttleKey::Reverse, tl);
        assert_eq!(t.state(), TransportState::Prerolling { rate: Rate(-1) });
    }

    #[test]
    fn pause_quantises_the_playhead_to_the_frame_start() {
        let tl = timeline(10);
        let t = transport();
        t.toggle(tl);
        show(&t, Time(7 * FRAME + FRAME / 3));
        t.pause(tl);
        assert_eq!(t.shared.playhead(), Time(7 * FRAME));
        assert_eq!(t.shared.playhead().flicks() % FRAME, 0);
        // A shown time past the end (the timeline was shortened) clamps to the last frame.
        t.toggle(tl);
        show(&t, Time::from_seconds(20));
        t.pause(tl);
        assert_eq!(t.shared.playhead(), tl.last_frame_start());
    }

    #[test]
    fn step_pauses_then_moves_whole_frames_clamped() {
        let tl = timeline(1);
        let t = transport();
        t.shared.seek(Time(5 * FRAME + 7));
        let generation = t.shared.seek_generation();
        let (g, effects) = t.step(1, tl);
        assert_eq!(g, generation + 1);
        assert_eq!(t.shared.playhead(), Time(6 * FRAME));
        assert_eq!(effects.seek, Some((g, SeekMode::Exact)));
        assert_eq!(t.step(-10, tl).0, g + 1);
        assert_eq!(
            t.shared.playhead(),
            Time::ZERO,
            "clamped at the first frame"
        );
        t.step(1000, tl);
        assert_eq!(
            t.shared.playhead(),
            Time(29 * FRAME),
            "clamped at the last frame"
        );
        assert_eq!(tl.last_frame(), 29);

        // From the end of the timeline one step back is the last frame.
        t.shared.seek(tl.duration);
        t.step(-1, tl);
        assert_eq!(t.shared.playhead(), Time(29 * FRAME));

        // While playing: pause on the shown frame first.
        t.toggle(timeline(10));
        show(&t, Time(12 * FRAME + 3));
        let (_, effects) = t.step(-1, timeline(10));
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(t.shared.playhead(), Time(11 * FRAME));
        assert_eq!(effects.audio[0], AudioCommand::Pause);
    }

    #[test]
    fn seeks_clamp_and_increase_the_generation() {
        let tl = timeline(10);
        let t = transport();
        let mut last = t.shared.seek_generation();
        for (time, expected) in [
            (Time::from_seconds(3), Time::from_seconds(3)),
            (Time::from_seconds(30), Time::from_seconds(10)),
            (Time(-5), Time::ZERO),
        ] {
            let (generation, effects) = t.seek(time, SeekMode::Exact, tl);
            assert!(generation > last);
            last = generation;
            assert_eq!(t.shared.playhead(), expected);
            assert_eq!(
                effects.audio,
                [AudioCommand::Seek {
                    to: expected,
                    seek_gen: generation,
                    mode: SeekMode::Exact
                }]
            );
            assert_eq!(t.state(), TransportState::Paused);
        }
    }

    #[test]
    fn exact_seek_while_playing_prerolls_again_scrub_pauses() {
        let tl = timeline(10);
        let t = transport();
        t.toggle(tl);
        t.preroll_done(t.shared.seek_generation());
        t.shuttle(ShuttleKey::Forward, tl);
        let (generation, effects) = t.seek(Time::from_seconds(6), SeekMode::Exact, tl);
        assert_eq!(t.state(), TransportState::Prerolling { rate: Rate(2) });
        assert!(!t.shared.started());
        assert_eq!(t.shared.seek_generation(), generation);
        assert_eq!(
            effects.audio,
            [AudioCommand::Play {
                from: Time::from_seconds(6),
                rate: Rate(2),
                seek_gen: generation
            }]
        );
        // A preroll that was replaced is not finished by its old generation.
        assert_eq!(t.preroll_done(generation - 1), Effects::none());
        assert_eq!(t.state(), TransportState::Prerolling { rate: Rate(2) });

        let (scrub, effects) = t.seek(Time::from_seconds(2), SeekMode::Scrub, tl);
        assert_eq!(scrub, generation + 1);
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(t.shared.playhead(), Time::from_seconds(2));
        assert_eq!(
            effects.audio,
            [
                AudioCommand::Pause,
                AudioCommand::Seek {
                    to: Time::from_seconds(2),
                    seek_gen: scrub,
                    mode: SeekMode::Scrub
                }
            ]
        );
    }

    #[test]
    fn preroll_then_end_pauses_on_the_last_frame() {
        let tl = timeline(10);
        let t = transport();
        t.toggle(tl);
        let generation = t.shared.seek_generation();
        assert!(t.preroll_done(generation).changed);
        assert!(t.shared.started());
        assert_eq!(t.state(), TransportState::Playing { rate: Rate(1) });
        // An end of an older run is ignored.
        assert_eq!(t.end(generation - 1, tl), Effects::none());
        let effects = t.end(generation, tl);
        assert_eq!(effects.audio, [AudioCommand::Pause]);
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(t.shared.playhead(), tl.last_frame_start());
        assert_eq!(tl.last_frame_start(), Time(299 * FRAME));
        assert!(!t.shared.started());
        // Ended twice: nothing.
        assert_eq!(t.end(generation, tl), Effects::none());

        // Reverse pauses at 0.
        t.shared.seek(Time::from_seconds(5));
        t.shuttle(ShuttleKey::Reverse, tl);
        t.end(t.shared.seek_generation(), tl);
        assert_eq!(t.shared.playhead(), Time::ZERO);
    }

    #[test]
    fn loop_and_project_replacement() {
        let tl = timeline(10);
        let mut t = transport();
        let effects = t.set_loop(true);
        assert!(t.shared.looping());
        assert_eq!(effects.audio, [AudioCommand::SetLooping(true)]);

        t.shared.seek(Time::from_seconds(3));
        t.toggle(tl);
        t.shared.set_skim(Some(Time(1)));
        let before = t.shared.seek_generation();
        let effects = t.reset();
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(t.shared.playhead(), Time::ZERO);
        assert_eq!(t.shared.seek_generation(), before + 1);
        assert_eq!(t.shared.skim(), None);
        assert_eq!(effects.audio[0], AudioCommand::Pause);
        assert!(t.shared.looping(), "loop is a session setting");
        // Paused already: still a new generation.
        t.reset();
        assert_eq!(t.shared.seek_generation(), before + 2);
    }

    #[test]
    fn play_clears_the_skim() {
        let tl = timeline(10);
        let t = transport();
        t.shared.set_skim(Some(Time::from_seconds(1)));
        t.toggle(tl);
        assert_eq!(t.shared.skim(), None);
    }

    #[test]
    fn skim_is_honoured_only_while_paused_and_play_clears_it() {
        let tl = timeline(10);
        let t = transport();
        t.shared.seek(Time::from_seconds(2));
        let generation = t.shared.seek_generation();
        assert!(t.set_skim(Some(Time::from_seconds(4)), tl));
        assert_eq!(t.shared.skim(), Some(Time::from_seconds(4)));
        assert!(!t.set_skim(Some(Time::from_seconds(4)), tl), "unchanged");
        // Clamped to the timeline; no generation, the playhead stays.
        assert!(t.set_skim(Some(Time::from_seconds(60)), tl));
        assert_eq!(t.shared.skim(), Some(Time::from_seconds(10)));
        assert_eq!(t.shared.playhead(), Time::from_seconds(2));
        assert_eq!(t.shared.seek_generation(), generation);

        // Play clears it; while playing a skim is ignored.
        t.toggle(tl);
        assert_eq!(t.shared.skim(), None);
        assert!(!t.set_skim(Some(Time::from_seconds(1)), tl));
        assert_eq!(t.shared.skim(), None);
        t.pause(tl);
        assert!(t.set_skim(Some(Time::from_seconds(1)), tl));
        assert!(t.set_skim(None, tl));
        assert_eq!(t.shared.skim(), None);
    }

    #[test]
    fn a_media_target_keeps_the_timeline_playhead_and_returns_to_it() {
        let tl = timeline(10);
        let media = timeline(3);
        let mut t = transport();
        t.shared.seek(Time::from_seconds(6));
        t.toggle(tl);
        show(&t, Time::from_seconds(7));
        t.set_skim(Some(Time(1)), tl);
        let before = t.shared.seek_generation();

        // To the media: pauses (on the frame on screen), starts at media time 0.
        let effects = t.retarget(tl, true, false, || media);
        assert_eq!(t.state(), TransportState::Paused);
        assert_eq!(effects.audio[0], AudioCommand::Pause);
        assert_eq!(t.shared.playhead(), Time::ZERO);
        assert_eq!(t.shared.skim(), None);
        assert_eq!(t.shared.seek_generation(), before + 1);
        assert_eq!(effects.seek, Some((before + 1, SeekMode::Exact)));

        // Transport commands act on media time.
        t.step(10, media);
        assert_eq!(t.shared.playhead(), Time(10 * FRAME));
        t.seek(Time::from_seconds(9), SeekMode::Exact, media);
        assert_eq!(t.shared.playhead(), Time::from_seconds(3));

        // Another media item: the timeline playhead is still the one kept.
        t.retarget(media, false, false, || media);
        assert_eq!(t.shared.playhead(), Time::ZERO);

        // Back to the timeline at the kept playhead (the frame shown when it paused).
        let effects = t.retarget(media, false, true, || tl);
        assert_eq!(t.shared.playhead(), Time::from_seconds(7));
        assert_eq!(t.shared.shown(), Time::from_seconds(7));
        assert!(effects.changed);

        // An edit made the timeline shorter meanwhile: clamped.
        t.retarget(tl, true, false, || media);
        t.retarget(media, false, true, || timeline(5));
        assert_eq!(t.shared.playhead(), Time::from_seconds(5));
    }
}
