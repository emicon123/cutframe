//! The seam between the transport and the audio path (playback.md §5, §6).
//!
//! The transport tells the audio path what changed ([`AudioCommand`], from the main thread or
//! the render thread, never blocking); the render thread asks it for the audible time (the
//! master clock), whether the ring is filled for preroll, and statistics for the present-rate
//! log. The real implementation is [`EngineAudio`] over the audio engine (feeder thread, `cpal`
//! stream, mixer, [`AudioEngine::clock`]). [`NoAudio`] is the silent fallback with a wall clock,
//! for tests and when the feeder thread cannot start.

use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use app_core::{FLICKS_PER_SECOND, Project, Time};
use arc_swap::ArcSwap;

use super::Playback;
use super::clock::ClockSource;
use super::transport::{AudioCommand, Rate};
use crate::audio::{AudioEngine, AudioEvent};
use crate::transport::{SeekMode, TransportShared};

/// The timeline time that is audible now, from the master clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audible {
    /// The generation the reading belongs to; readings of an older one are ignored.
    pub seek_gen: u32,
    /// Incremented at every loop wrap within one generation.
    pub epoch: u32,
    pub time: Time,
    /// The run ended: the end of the timeline (forward, without loop) or 0 (reverse).
    pub ended: bool,
}

/// Audio numbers for the present-rate log (playback.md §11).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioStats {
    /// Device buffers that found the ring empty, since the stream opened.
    pub underruns: u32,
    /// Lowest ring fill in blocks since the last call.
    pub ring_min_fill: u32,
}

/// What the transport needs from the audio path. Every method is cheap and never blocks:
/// `command` is called on the main thread (and on the render thread at the end of playback),
/// the others on the render thread.
pub trait AudioSink: Send + Sync {
    /// A transport change, in the order the transport made them.
    fn command(&self, command: AudioCommand);

    /// Preroll: the ring holds its target fill for the current generation (always true when
    /// there is no sound to wait for).
    fn ring_ready(&self) -> bool;

    /// The audible timeline time, or `None` before the first sample of the current generation
    /// reached the device (preroll) or while paused. Clamped to the last real sample, so an
    /// underrun freezes it and the end of the timeline stops it at `duration`.
    fn audible(&self) -> Option<Audible>;

    fn stats(&self) -> AudioStats {
        AudioStats::default()
    }

    /// Called once with the transport this sink serves, so it can pause it (device lost).
    fn attach(&self, playback: Weak<Playback>) {
        let _ = playback;
    }
}

/// The audio engine as the transport's audio path: sound at 1x, the device clock (or the
/// engine's null output at wall-clock pace without a device), scrub grains. A lost output
/// device pauses the transport; the next play opens the default device again.
pub struct EngineAudio {
    engine: AudioEngine,
    /// Kept for the engine's life (one clock for device and null output).
    clock: Arc<dyn ClockSource>,
    playback: Arc<OnceLock<Weak<Playback>>>,
}

impl std::fmt::Debug for EngineAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineAudio")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

impl EngineAudio {
    /// Starts the audio engine (its feeder thread) for `transport` and `snapshot`.
    pub fn new(
        transport: Arc<TransportShared>,
        snapshot: Arc<ArcSwap<Project>>,
    ) -> std::io::Result<Self> {
        let playback: Arc<OnceLock<Weak<Playback>>> = Arc::new(OnceLock::new());
        let attached = Arc::clone(&playback);
        let engine = AudioEngine::new(transport, snapshot, move |event| match event {
            AudioEvent::DeviceLost => {
                tracing::warn!("audio output device lost; pausing playback");
                if let Some(playback) = attached.get().and_then(Weak::upgrade) {
                    playback.pause();
                }
            }
        })?;
        Ok(Self {
            clock: engine.clock(),
            engine,
            playback,
        })
    }

    pub fn engine(&self) -> &AudioEngine {
        &self.engine
    }
}

impl AudioSink for EngineAudio {
    fn command(&self, command: AudioCommand) {
        match command {
            AudioCommand::Play {
                from,
                rate,
                seek_gen,
            } => self.engine.play(from, rate.get(), seek_gen),
            AudioCommand::SetRate(rate) => self.engine.set_rate(rate.get()),
            AudioCommand::Pause => self.engine.pause(),
            AudioCommand::Seek {
                to,
                seek_gen,
                mode: SeekMode::Scrub,
            } => self.engine.scrub(to, seek_gen),
            AudioCommand::Seek { to, seek_gen, mode } => self.engine.seek(to, seek_gen, mode),
            AudioCommand::SetLooping(on) => self.engine.set_looping(on),
        }
    }

    fn ring_ready(&self) -> bool {
        self.engine.ring_ready()
    }

    fn audible(&self) -> Option<Audible> {
        self.clock.now().map(|reading| Audible {
            seek_gen: reading.seek_gen,
            epoch: reading.epoch,
            time: reading.time,
            ended: reading.ended,
        })
    }

    fn stats(&self) -> AudioStats {
        AudioStats {
            underruns: u32::try_from(self.engine.underruns()).unwrap_or(u32::MAX),
            ring_min_fill: self.engine.take_ring_min_fill().unwrap_or(0),
        }
    }

    fn attach(&self, playback: Weak<Playback>) {
        let _ = self.playback.set(playback);
    }
}

/// `duration` in flicks, exact to the nanosecond (integer arithmetic).
pub fn flicks_of(duration: Duration) -> Time {
    let flicks = duration.as_nanos() * FLICKS_PER_SECOND as u128 / 1_000_000_000;
    Time(i64::try_from(flicks).unwrap_or(i64::MAX))
}

/// The wall-clock fallback: no sound; the clock runs from the moment the render thread ends
/// preroll (`TransportShared::started`), wraps at the end while looping and stops at the end
/// (forward) or at 0 (reverse) otherwise.
pub struct NoAudio {
    transport: Arc<TransportShared>,
    snapshot: Arc<ArcSwap<Project>>,
    run: Mutex<Option<Run>>,
}

impl std::fmt::Debug for NoAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoAudio").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
struct Run {
    seek_gen: u32,
    epoch: u32,
    rate: Rate,
    /// Timeline time at `since`.
    from: Time,
    /// When the clock started (or was last rebased); `None` until preroll ended.
    since: Option<Instant>,
}

impl Run {
    fn time_at(&self, now: Instant) -> Time {
        let Some(since) = self.since else {
            return self.from;
        };
        let elapsed = flicks_of(now.saturating_duration_since(since));
        self.from + Time(elapsed.flicks().saturating_mul(i64::from(self.rate.get())))
    }

    fn rebase(&mut self, now: Instant) {
        self.from = self.time_at(now);
        if self.since.is_some() {
            self.since = Some(now);
        }
    }
}

impl NoAudio {
    pub fn new(transport: Arc<TransportShared>, snapshot: Arc<ArcSwap<Project>>) -> Self {
        Self {
            transport,
            snapshot,
            run: Mutex::new(None),
        }
    }

    fn run(&self) -> std::sync::MutexGuard<'_, Option<Run>> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The reading at `now` (tests pass their own instants).
    pub fn audible_at(&self, now: Instant) -> Option<Audible> {
        let mut guard = self.run();
        let run = guard.as_mut()?;
        if !self.transport.started() || run.seek_gen != self.transport.seek_generation() {
            return None;
        }
        if run.since.is_none() {
            run.since = Some(now);
        }
        let duration = self.snapshot.load().duration();
        let mut time = run.time_at(now);
        if run.rate.is_forward() && time >= duration {
            if self.transport.looping() && duration > Time::ZERO {
                // Wrap: the next loop starts where the overshoot ends.
                let overshoot = Time((time - duration).flicks() % duration.flicks());
                run.epoch += 1;
                run.from = overshoot;
                run.since = Some(now);
                time = overshoot;
            } else {
                time = duration;
            }
        } else if !run.rate.is_forward() && time <= Time::ZERO {
            time = Time::ZERO;
        }
        let ended = if run.rate.is_forward() {
            !self.transport.looping() && time >= duration
        } else {
            time <= Time::ZERO
        };
        Some(Audible {
            seek_gen: run.seek_gen,
            epoch: run.epoch,
            time,
            ended,
        })
    }
}

impl AudioSink for NoAudio {
    fn command(&self, command: AudioCommand) {
        let mut run = self.run();
        match command {
            AudioCommand::Play {
                from,
                rate,
                seek_gen,
            } => {
                *run = Some(Run {
                    seek_gen,
                    epoch: 0,
                    rate,
                    from,
                    since: None,
                });
            }
            AudioCommand::SetRate(rate) => {
                if let Some(run) = run.as_mut() {
                    run.rebase(Instant::now());
                    run.rate = rate;
                }
            }
            AudioCommand::Pause => *run = None,
            AudioCommand::Seek { .. } | AudioCommand::SetLooping(_) => {}
        }
    }

    fn ring_ready(&self) -> bool {
        true
    }

    fn audible(&self) -> Option<Audible> {
        self.audible_at(Instant::now())
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn flicks_of_durations_are_exact() {
        assert_eq!(flicks_of(Duration::from_secs(1)), Time::from_seconds(1));
        assert_eq!(
            flicks_of(Duration::from_millis(500)),
            Time(FLICKS_PER_SECOND / 2)
        );
        assert_eq!(flicks_of(Duration::from_nanos(1)), Time(0), "0.7 flicks");
    }

    #[test]
    fn the_wall_clock_starts_with_preroll_and_follows_the_rate() {
        let transport = Arc::new(TransportShared::new());
        let audio = NoAudio::new(Arc::clone(&transport), project(10));
        let start = Instant::now();
        let generation = transport.seek(Time::from_seconds(2));
        audio.command(AudioCommand::Play {
            from: Time::from_seconds(2),
            rate: Rate::FORWARD,
            seek_gen: generation,
        });
        assert_eq!(audio.audible_at(start), None, "preroll");
        transport.set_started(true);
        let first = audio.audible_at(start).expect("started");
        assert_eq!(first.time, Time::from_seconds(2));
        let later = audio.audible_at(start + Duration::from_secs(1)).unwrap();
        assert_eq!(later.time, Time::from_seconds(3));

        audio.command(AudioCommand::SetRate(Rate::new(4).unwrap()));
        let now = Instant::now();
        let base = audio.audible_at(now).unwrap().time;
        let fast = audio.audible_at(now + Duration::from_millis(250)).unwrap();
        assert_eq!(fast.time - base, Time::from_seconds(1));

        // An older generation's run gives no reading.
        transport.seek(Time::ZERO);
        assert_eq!(audio.audible_at(now), None);
    }

    #[test]
    fn the_wall_clock_wraps_when_looping_and_stops_at_the_end_otherwise() {
        let transport = Arc::new(TransportShared::new());
        let audio = NoAudio::new(Arc::clone(&transport), project(2));
        let start = Instant::now();
        let generation = transport.seek(Time::from_seconds(1));
        audio.command(AudioCommand::Play {
            from: Time::from_seconds(1),
            rate: Rate::FORWARD,
            seek_gen: generation,
        });
        transport.set_started(true);
        audio.audible_at(start);
        let end = audio.audible_at(start + Duration::from_secs(5)).unwrap();
        assert_eq!(end.time, Time::from_seconds(2), "frozen at the end");
        assert_eq!(end.epoch, 0);

        transport.set_looping(true);
        let wrapped = audio
            .audible_at(start + Duration::from_millis(1500))
            .unwrap();
        assert_eq!(wrapped.epoch, 1);
        assert_eq!(wrapped.time, Time(FLICKS_PER_SECOND / 2));
        let next = audio.audible_at(start + Duration::from_secs(2)).unwrap();
        assert_eq!((next.epoch, next.time), (1, Time::from_seconds(1)));

        // Reverse stops at 0.
        let generation = transport.seek(Time::from_seconds(1));
        audio.command(AudioCommand::Play {
            from: Time::from_seconds(1),
            rate: Rate::REVERSE,
            seek_gen: generation,
        });
        audio.audible_at(start);
        let zero = audio.audible_at(start + Duration::from_secs(3)).unwrap();
        assert_eq!(zero.time, Time::ZERO);
        audio.command(AudioCommand::Pause);
        assert_eq!(audio.audible_at(start), None);
    }
}
