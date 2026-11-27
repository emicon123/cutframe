//! Playback (playback.md): the transport state machine, the seam to the audio path, frame
//! pacing and the decode-ahead lookahead.
//!
//! [`Playback`] is the transport's front door: the shell's commands call it on the main thread,
//! the render thread calls it when preroll ends and when playback reaches the end. Each call
//! takes a short mutex around the pure [`transport::Transport`] (microseconds, never contended
//! for long), writes the [`TransportShared`] atomics the engine threads read lock-free, tells
//! the [`AudioSink`] and wakes the render thread, which redraws and emits a `playback-tick`.
//!
//! What plays is a [`PlaySource`] (task 0340): the timeline, or one media item (source preview).
//! Every range, clamp and frame grid of the transport comes from its project, so the transport
//! commands act on media time while a media target plays.

pub mod audio_sink;
pub mod clock;
pub mod pacing;
pub mod scheduler;
pub mod source;
pub mod transport;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use app_core::Time;

pub use self::audio_sink::{Audible, AudioSink, AudioStats, EngineAudio, NoAudio};
pub use self::source::{PlaySource, PlaybackTarget, ProjectSource, UnknownMedia};
pub use self::transport::{
    AudioCommand, Effects, Rate, ShuttleKey, Timeline, Transport, TransportState,
};
use crate::transport::{PlayState, SeekMode, TransportShared};

/// What the transport tells the render thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportWake {
    /// State, rate or loop changed.
    Changed,
    /// The playhead moved (a seek, step, target change or project replacement) to generation
    /// `generation`.
    Seek { generation: u32, mode: SeekMode },
    /// The skim time changed: redraw, no tick.
    Skim,
}

/// The transport as the UI sees it: the payload of `playback-tick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportStatus {
    /// The frame on screen while playing (its start), the playhead while paused; media time
    /// while a media target plays. Never the skim time.
    pub time: Time,
    pub state: PlayState,
    /// -8..=8, 0 while paused.
    pub rate: i8,
    pub looping: bool,
    /// The newest seek generation.
    pub seek_gen: u32,
    /// Dropped frames since the last play start.
    pub dropped_frames: u32,
    /// What plays.
    pub target: PlaybackTarget,
}

/// Called on the render thread with every tick (at most 60 Hz while playing, once after every
/// change). Must not block.
pub type PlaybackTicks = Box<dyn Fn(TransportStatus) + Send>;

/// The transport front door. See the module docs.
pub struct Playback {
    transport: Mutex<Transport>,
    shared: Arc<TransportShared>,
    played: Arc<PlaySource>,
    audio: Arc<dyn AudioSink>,
    wake: Box<dyn Fn(TransportWake) + Send + Sync>,
    dropped: AtomicU32,
}

impl std::fmt::Debug for Playback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Playback")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Playback {
    /// A transport over `shared` for what `played` plays. `wake` reaches the render thread;
    /// `audio` is the audio path ([`NoAudio`] without sound), reading the same `played`.
    pub fn new(
        shared: Arc<TransportShared>,
        played: Arc<PlaySource>,
        audio: Arc<dyn AudioSink>,
        wake: Box<dyn Fn(TransportWake) + Send + Sync>,
    ) -> Self {
        Self {
            transport: Mutex::new(Transport::new(Arc::clone(&shared))),
            shared,
            played,
            audio,
            wake,
            dropped: AtomicU32::new(0),
        }
    }

    pub fn shared(&self) -> &Arc<TransportShared> {
        &self.shared
    }

    pub fn audio(&self) -> &Arc<dyn AudioSink> {
        &self.audio
    }

    /// What plays (the render thread and the feeder read the same).
    pub fn played(&self) -> &Arc<PlaySource> {
        &self.played
    }

    /// The range and frame grid of what plays.
    fn timeline(&self) -> Timeline {
        Timeline::of(&self.played.project())
    }

    fn lock(&self) -> MutexGuard<'_, Transport> {
        // The transport is a view over atomics: always consistent.
        self.transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `input` under the lock, tells the audio path in order, then wakes the render
    /// thread (outside the lock).
    fn apply<T>(&self, input: impl FnOnce(&mut Transport, Timeline) -> (T, Effects)) -> T {
        let timeline = self.timeline();
        let (value, effects) = {
            let mut transport = self.lock();
            let (value, effects) = input(&mut transport, timeline);
            for command in &effects.audio {
                self.audio.command(*command);
            }
            (value, effects)
        };
        if effects
            .audio
            .iter()
            .any(|command| matches!(command, AudioCommand::Play { .. }))
        {
            self.dropped.store(0, Ordering::Relaxed);
        }
        if let Some((generation, mode)) = effects.seek {
            (self.wake)(TransportWake::Seek { generation, mode });
        } else if effects.changed {
            (self.wake)(TransportWake::Changed);
        }
        value
    }

    /// Space: play at 1x (from 0 at the end) or pause.
    pub fn toggle(&self) {
        self.apply(|t, tl| ((), t.toggle(tl)));
    }

    pub fn pause(&self) {
        self.apply(|t, tl| ((), t.pause(tl)));
    }

    pub fn shuttle(&self, key: ShuttleKey) {
        self.apply(|t, tl| ((), t.shuttle(key, tl)));
    }

    /// Pauses and moves `frames` frames; returns the new seek generation.
    pub fn step(&self, frames: i32) -> u32 {
        self.apply(|t, tl| t.step(frames, tl))
    }

    /// Seeks (clamped to the timeline); returns the new seek generation.
    pub fn seek(&self, time: Time, mode: SeekMode) -> u32 {
        self.apply(|t, tl| t.seek(time, mode, tl))
    }

    pub fn set_loop(&self, on: bool) {
        self.apply(|t, _| ((), t.set_loop(on)));
    }

    /// The project was replaced (new, open, restore): the timeline plays again, paused at 0, a
    /// new generation.
    pub fn reset(&self) {
        self.apply(|t, _| {
            let _ = self.played.set_target(PlaybackTarget::Timeline);
            ((), t.reset())
        });
    }

    /// Skim (playback.md §4.2, task 0340): while paused, the preview shows `time` (clamped to
    /// the range of what plays) instead of the playhead, without sound, a new generation or a
    /// tick; `None` shows the playhead again. Ignored while playing; play clears it.
    pub fn set_skim(&self, time: Option<Time>) {
        let timeline = self.timeline();
        let changed = self.lock().set_skim(time, timeline);
        if changed {
            (self.wake)(TransportWake::Skim);
        }
    }

    /// Switches what plays (playback.md §8, task 0340): pauses, keeps the timeline playhead
    /// when leaving the timeline, and starts a media target at media time 0 or the timeline at
    /// its kept playhead (a new generation, the skim cleared). Returns the seek generation. The
    /// same target again changes nothing (and returns the current generation). Media that is
    /// not in the project is an error and changes nothing.
    pub fn set_target(&self, target: PlaybackTarget) -> Result<u32, UnknownMedia> {
        let before = self.timeline();
        let effects = {
            let mut transport = self.lock();
            let current = self.played.target();
            if current == target {
                return Ok(self.shared.seek_generation());
            }
            let prepared = self.played.prepare(target)?;
            let effects = transport.retarget(
                before,
                current == PlaybackTarget::Timeline,
                target == PlaybackTarget::Timeline,
                || {
                    self.played.switch(prepared);
                    self.timeline()
                },
            );
            for command in &effects.audio {
                self.audio.command(*command);
            }
            effects
        };
        let (generation, mode) = effects.seek.expect("a target change seeks");
        (self.wake)(TransportWake::Seek { generation, mode });
        Ok(generation)
    }

    /// Render thread: preroll of `seek_gen` finished (first frame and ring ready, or the
    /// timeout): the audio callback may consume the ring.
    pub fn preroll_done(&self, seek_gen: u32) {
        self.apply(|t, _| ((), t.preroll_done(seek_gen)));
    }

    /// Render thread: playback of `seek_gen` reached its end.
    pub fn end(&self, seek_gen: u32) {
        self.apply(|t, tl| ((), t.end(seek_gen, tl)));
    }

    pub(crate) fn set_dropped(&self, dropped: u32) {
        self.dropped.store(dropped, Ordering::Relaxed);
    }

    /// The transport now (`get_playback`, ticks). Lock-free.
    pub fn status(&self) -> TransportStatus {
        let state = self.shared.state();
        TransportStatus {
            time: match state {
                PlayState::Paused => self.shared.playhead(),
                PlayState::Prerolling | PlayState::Playing => self.shared.shown(),
            },
            state,
            rate: self.shared.rate(),
            looping: self.shared.looping(),
            seek_gen: self.shared.seek_generation(),
            dropped_frames: self.dropped.load(Ordering::Relaxed),
            target: self.played.target(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use app_core::Project;
    use arc_swap::ArcSwap;

    use super::*;

    fn project(seconds: i64) -> Arc<ArcSwap<Project>> {
        let mut project = Project::new("Test");
        project.tracks[0].clips.push(app_core::Clip::color(
            "C",
            Time::ZERO,
            Time::from_seconds(seconds),
            app_core::Rgba8::BLACK,
        ));
        Arc::new(ArcSwap::from_pointee(project))
    }

    #[derive(Default)]
    struct Recorder(StdMutex<Vec<AudioCommand>>);

    impl AudioSink for Recorder {
        fn command(&self, command: AudioCommand) {
            self.0.lock().unwrap().push(command);
        }
        fn ring_ready(&self) -> bool {
            true
        }
        fn audible(&self) -> Option<Audible> {
            None
        }
    }

    #[test]
    fn commands_reach_the_audio_path_and_wake_the_render_thread() {
        let audio = Arc::new(Recorder::default());
        let wakes = Arc::new(StdMutex::new(Vec::new()));
        let log = Arc::clone(&wakes);
        let playback = Playback::new(
            Arc::new(TransportShared::new()),
            Arc::new(PlaySource::new(project(10))),
            audio.clone(),
            Box::new(move |wake| log.lock().unwrap().push(wake)),
        );
        playback.toggle();
        let generation = playback.status().seek_gen;
        assert_eq!(playback.status().state, PlayState::Prerolling);
        playback.set_dropped(3);
        playback.preroll_done(generation);
        assert_eq!(playback.status().state, PlayState::Playing);
        assert_eq!(playback.status().dropped_frames, 3);
        playback.shared.set_shown(Time::from_seconds(2));
        assert_eq!(playback.status().time, Time::from_seconds(2));
        let step = playback.step(1);
        assert_eq!(step, generation + 1);
        assert_eq!(playback.status().state, PlayState::Paused);
        playback.set_loop(true);
        playback.toggle();
        assert_eq!(playback.status().dropped_frames, 0, "reset at play start");
        playback.end(playback.status().seek_gen);
        assert_eq!(
            playback.status().time,
            Time::from_seconds(10) - Time(705_600_000 / 30)
        );
        playback.reset();
        assert_eq!(playback.status().time, Time::ZERO);

        let commands = audio.0.lock().unwrap().clone();
        assert!(matches!(commands[0], AudioCommand::Play { .. }));
        assert_eq!(commands[1], AudioCommand::Pause, "step pauses first");
        assert!(matches!(commands[2], AudioCommand::Seek { .. }));
        assert_eq!(commands[3], AudioCommand::SetLooping(true));
        let wakes = wakes.lock().unwrap().clone();
        assert_eq!(wakes[0], TransportWake::Changed);
        assert_eq!(
            wakes[2],
            TransportWake::Seek {
                generation: step,
                mode: SeekMode::Exact
            }
        );
        assert!(matches!(wakes.last(), Some(TransportWake::Seek { .. })));
    }

    /// A transport over a 10 s timeline that also holds a 5 s video (`samples::video`), and its
    /// wakes.
    fn with_media() -> (
        Playback,
        app_core::MediaId,
        Arc<StdMutex<Vec<TransportWake>>>,
    ) {
        let snapshot = project(10);
        let video = app_core::media::samples::video(1);
        let id = video.id;
        let mut timeline = Project::clone(&snapshot.load());
        timeline.media.push(video);
        snapshot.store(Arc::new(timeline));
        let wakes = Arc::new(StdMutex::new(Vec::new()));
        let log = Arc::clone(&wakes);
        let shared = Arc::new(TransportShared::new());
        let played = Arc::new(PlaySource::new(snapshot));
        let audio = Arc::new(NoAudio::new(Arc::clone(&shared), played.clone()));
        let playback = Playback::new(
            shared,
            played,
            audio,
            Box::new(move |wake| log.lock().unwrap().push(wake)),
        );
        (playback, id, wakes)
    }

    #[test]
    fn a_media_target_plays_media_time_and_the_timeline_playhead_returns() {
        let (playback, media, wakes) = with_media();
        playback.seek(Time::from_seconds(8), SeekMode::Exact);
        assert_eq!(playback.status().target, PlaybackTarget::Timeline);

        let unknown = app_core::MediaId::new();
        assert_eq!(
            playback.set_target(PlaybackTarget::Media(unknown)),
            Err(UnknownMedia(unknown))
        );
        assert_eq!(playback.status().time, Time::from_seconds(8));

        let before = playback.status().seek_gen;
        assert_eq!(
            playback.set_target(PlaybackTarget::Media(media)),
            Ok(before + 1)
        );
        let status = playback.status();
        assert_eq!(status.target, PlaybackTarget::Media(media));
        assert_eq!(status.time, Time::ZERO);
        assert_eq!(status.seek_gen, before + 1);
        assert_eq!(
            wakes.lock().unwrap().last(),
            Some(&TransportWake::Seek {
                generation: before + 1,
                mode: SeekMode::Exact
            })
        );
        // The same target again: nothing.
        assert_eq!(
            playback.set_target(PlaybackTarget::Media(media)),
            Ok(before + 1)
        );
        assert_eq!(playback.status().seek_gen, before + 1);

        // Seeks clamp to the media's 5 s, steps move by its 30 fps frames.
        playback.seek(Time::from_seconds(9), SeekMode::Exact);
        assert_eq!(playback.status().time, Time::from_seconds(5));
        playback.step(-30);
        assert_eq!(playback.status().time, Time::from_seconds(4));
        playback.toggle();
        assert_eq!(playback.status().state, PlayState::Prerolling);
        assert_eq!(playback.status().target, PlaybackTarget::Media(media));

        playback.set_target(PlaybackTarget::Timeline).unwrap();
        let status = playback.status();
        assert_eq!(status.state, PlayState::Paused, "a target change pauses");
        assert_eq!(status.target, PlaybackTarget::Timeline);
        assert_eq!(status.time, Time::from_seconds(8));

        // A new project plays the timeline again.
        playback.set_target(PlaybackTarget::Media(media)).unwrap();
        playback.reset();
        assert_eq!(playback.status().target, PlaybackTarget::Timeline);
        assert_eq!(playback.status().time, Time::ZERO);
    }

    #[test]
    fn skim_wakes_without_a_tick_only_while_paused() {
        let (playback, _, wakes) = with_media();
        let shared = Arc::clone(playback.shared());
        playback.seek(Time::from_seconds(2), SeekMode::Exact);
        wakes.lock().unwrap().clear();
        playback.set_skim(Some(Time::from_seconds(3)));
        assert_eq!(shared.skim(), Some(Time::from_seconds(3)));
        assert_eq!(*wakes.lock().unwrap(), [TransportWake::Skim]);
        assert_eq!(
            playback.status().time,
            Time::from_seconds(2),
            "ticks report the playhead"
        );
        playback.set_skim(Some(Time::from_seconds(3)));
        assert_eq!(wakes.lock().unwrap().len(), 1, "unchanged: no wake");

        playback.toggle();
        assert_eq!(shared.skim(), None, "playing clears it");
        wakes.lock().unwrap().clear();
        playback.set_skim(Some(Time::from_seconds(1)));
        assert_eq!(shared.skim(), None);
        assert!(wakes.lock().unwrap().is_empty());
    }
}
