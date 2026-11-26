//! Which project frame each vsync shows during playback, when playback ends, the refresh
//! estimate behind the present lead, and the `playback-tick` throttle (playback.md §5.2–§5.4,
//! §8). Pure logic, driven by the render thread and by the simulation tests.

use std::time::{Duration, Instant};

use app_core::Time;

use super::audio_sink::{Audible, flicks_of};
use super::transport::{Rate, Timeline};

/// The present lead in refresh intervals: one frame queued in the swap chain plus the DWM
/// compositor frame (playback.md §5.2).
pub const PRESENT_LEAD_REFRESHES: u32 = 2;

/// The refresh interval assumed until presents have been measured (60 Hz).
pub const DEFAULT_REFRESH: Duration = Duration::from_micros(16_667);

/// Ticks are throttled to at most this rate on average (ADR-0003).
pub const TICK_INTERVAL: Duration = Duration::from_micros(16_667);

/// How long preroll waits for the first frame and the audio ring before playback starts anyway.
pub const PREROLL_TIMEOUT: Duration = Duration::from_millis(300);

/// The frame a vsync shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    /// The project frame index.
    pub frame: i64,
    /// Its start time.
    pub time: Time,
    /// `t_show` before quantisation (for the A/V offset log).
    pub t_show: Time,
}

/// Chooses the frame per vsync: `t_show = t_audible + lead x rate`, never moving backwards
/// within one `(seek_gen, epoch)` at positive rates, `n = floor(t_show / frame)` (playback.md
/// §5.2). While looping, a `t_show` past the end shows the start of the next loop.
#[derive(Debug, Default, Clone)]
pub struct FramePicker {
    last: Option<(u32, u32, Time)>,
}

impl FramePicker {
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn pick(
        &mut self,
        audible: Audible,
        lead: Duration,
        rate: Rate,
        timeline: Timeline,
        looping: bool,
    ) -> Pick {
        let lead = flicks_of(lead)
            .flicks()
            .saturating_mul(i64::from(rate.get()));
        let mut t_show = audible.time + Time(lead);
        let key = (audible.seek_gen, audible.epoch);
        if rate.is_forward()
            && let Some((seek_gen, epoch, last)) = self.last
            && (seek_gen, epoch) == key
        {
            t_show = t_show.max(last);
        }
        self.last = Some((key.0, key.1, t_show));
        let duration = timeline.duration;
        let shown = if looping && duration > Time::ZERO && t_show >= duration {
            Time(t_show.flicks().rem_euclid(duration.flicks()))
        } else {
            t_show
        };
        let frame = shown
            .to_frames(timeline.frame_rate)
            .clamp(0, timeline.last_frame());
        Pick {
            frame,
            time: Time::from_frames(frame, timeline.frame_rate),
            t_show,
        }
    }
}

/// Playback of this reading is over: the clock says so, or forward without loop at the end
/// (the clock stops there), reverse at 0.
pub fn reached_end(audible: Audible, rate: Rate, timeline: Timeline, looping: bool) -> bool {
    if audible.ended {
        true
    } else if rate.is_forward() {
        !looping && audible.time >= timeline.duration
    } else {
        audible.time <= Time::ZERO
    }
}

/// The refresh interval, from the intervals between presents (an exponential average that
/// ignores pauses and missed vsyncs).
#[derive(Debug, Clone)]
pub struct RefreshEstimate {
    interval: Duration,
    last: Option<Instant>,
}

impl Default for RefreshEstimate {
    fn default() -> Self {
        Self {
            interval: DEFAULT_REFRESH,
            last: None,
        }
    }
}

impl RefreshEstimate {
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// The present lead (playback.md §5.2).
    pub fn lead(&self) -> Duration {
        self.interval * PRESENT_LEAD_REFRESHES
    }

    /// Presents stopped (paused, hidden): the next present starts a new measurement.
    pub fn pause(&mut self) {
        self.last = None;
    }

    pub fn presented(&mut self, now: Instant) {
        if let Some(last) = self.last.replace(now) {
            let gap = now.saturating_duration_since(last);
            // Up to 360 Hz; a gap much longer than the estimate is a missed vsync.
            let plausible =
                Duration::from_micros(2_700)..=self.interval * 3 / 2 + Duration::from_millis(1);
            if plausible.contains(&gap) {
                // 1/8 of the new sample per present.
                self.interval = (self.interval * 7 + gap) / 8;
            }
        }
    }
}

/// What the tick throttle says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickDue {
    /// Emit a tick now.
    Now,
    /// A tick is pending: emit it after this long.
    After(Duration),
    /// Nothing to send.
    Idle,
}

/// The `playback-tick` throttle (playback.md §8): while playing, ticks at most every
/// [`TICK_INTERVAL`] on average; after a state change one tick always follows, within one
/// interval, so the last state always arrives.
#[derive(Debug, Clone, Default)]
pub struct TickThrottle {
    /// The earliest time of the next tick.
    due: Option<Instant>,
    pending: bool,
}

impl TickThrottle {
    /// A state change, seek, step or loop toggle: the next tick must go out.
    pub fn request(&mut self) {
        self.pending = true;
    }

    pub fn poll(&self, now: Instant, playing: bool) -> TickDue {
        if !self.pending && !playing {
            return TickDue::Idle;
        }
        match self.due {
            Some(due) if due > now => TickDue::After(due - now),
            _ => TickDue::Now,
        }
    }

    /// A tick went out at `now`.
    pub fn emitted(&mut self, now: Instant) {
        self.pending = false;
        // Keep the cadence (late presents do not lower the average rate), but never bank
        // ticks over an idle stretch.
        self.due = Some(match self.due {
            Some(due) if now.saturating_duration_since(due) < TICK_INTERVAL => due + TICK_INTERVAL,
            _ => now + TICK_INTERVAL,
        });
    }
}

#[cfg(test)]
mod tests {
    use app_core::{FLICKS_PER_SECOND, FrameRate};

    use super::*;

    fn timeline(seconds: i64, frame_rate: FrameRate) -> Timeline {
        Timeline {
            duration: Time::from_seconds(seconds),
            frame_rate,
        }
    }

    fn audible(seek_gen: u32, epoch: u32, time: Time) -> Audible {
        Audible {
            seek_gen,
            epoch,
            time,
            ended: false,
        }
    }

    /// A fake device clock: the audible time at wall time `wall`, with a fixed latency and a
    /// deterministic jitter of up to +-`jitter` (an LCG), as a real callback publishing samples
    /// per buffer would look to the render thread.
    struct FakeClock {
        latency: Time,
        jitter: i64,
        seed: u64,
    }

    impl FakeClock {
        fn audible_at(&mut self, wall: Time) -> Time {
            self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let noise = (self.seed >> 33) as i64 % (2 * self.jitter + 1) - self.jitter;
            Time((wall - self.latency).flicks().max(0) + noise).max(Time::ZERO)
        }
    }

    const RATES: [FrameRate; 5] = [
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::NTSC_30,
        FrameRate::FPS_30,
        FrameRate::FPS_60,
    ];

    /// 20 s of playback per project rate and refresh rate: the chosen frame is
    /// `floor((t_audible + lead) / frame)` (or the monotonic hold), never goes back, and the end
    /// is detected when the clock reaches the duration, with the last frame shown.
    #[test]
    fn frames_follow_the_clock_and_never_go_back() {
        for refresh_hz in [60u64, 144, 165] {
            let vsync = Duration::from_nanos(1_000_000_000 / refresh_hz);
            let lead = vsync * PRESENT_LEAD_REFRESHES;
            for frame_rate in RATES {
                let tl = timeline(20, frame_rate);
                let frame = frame_rate.frame_duration().flicks();
                let mut clock = FakeClock {
                    latency: Time(FLICKS_PER_SECOND / 50),
                    jitter: FLICKS_PER_SECOND / 2000,
                    seed: refresh_hz,
                };
                let mut picker = FramePicker::default();
                let mut last_frame = -1;
                let mut ended = false;
                for n in 0.. {
                    let wall = flicks_of(vsync * n);
                    let now = audible(1, 0, clock.audible_at(wall).min(tl.duration));
                    if reached_end(now, Rate::FORWARD, tl, false) {
                        ended = true;
                        break;
                    }
                    let pick = picker.pick(now, lead, Rate::FORWARD, tl, false);
                    let expected =
                        ((now.time + flicks_of(lead)).flicks() / frame).min(tl.last_frame());
                    assert!(
                        pick.frame == expected || pick.frame == last_frame,
                        "{refresh_hz} Hz {frame_rate:?}: vsync {n} picked {} expected {expected}",
                        pick.frame
                    );
                    assert!(pick.frame >= last_frame, "backwards at vsync {n}");
                    // With a display faster than the project no frame is skipped (the first
                    // pick may already be past frame 0: preroll showed it).
                    if last_frame >= 0 && refresh_hz as f64 > frame_rate.as_f64() {
                        assert!(pick.frame - last_frame <= 1, "skipped at vsync {n}");
                    }
                    last_frame = pick.frame;
                    assert_eq!(pick.time, Time::from_frames(pick.frame, frame_rate));
                }
                assert!(ended);
                assert_eq!(last_frame, tl.last_frame(), "the last frame was shown");
            }
        }
    }

    #[test]
    fn jitter_backwards_holds_the_frame_within_one_generation() {
        let tl = timeline(10, FrameRate::FPS_30);
        let mut picker = FramePicker::default();
        let lead = Duration::ZERO;
        let frame = FrameRate::FPS_30.frame_duration();
        let a = picker.pick(
            audible(1, 0, Time(5 * frame.flicks())),
            lead,
            Rate::FORWARD,
            tl,
            false,
        );
        let b = picker.pick(
            audible(1, 0, Time(5 * frame.flicks() - 10)),
            lead,
            Rate::FORWARD,
            tl,
            false,
        );
        assert_eq!((a.frame, b.frame), (5, 5));
        // A new generation (a seek) or epoch (a loop wrap) may go back.
        let c = picker.pick(audible(2, 0, Time::ZERO), lead, Rate::FORWARD, tl, false);
        assert_eq!(c.frame, 0);
        // Reverse goes back by design.
        let d = picker.pick(
            audible(3, 0, Time(9 * frame.flicks())),
            lead,
            Rate::REVERSE,
            tl,
            false,
        );
        let e = picker.pick(
            audible(3, 0, Time(8 * frame.flicks())),
            lead,
            Rate::REVERSE,
            tl,
            false,
        );
        assert_eq!((d.frame, e.frame), (9, 8));
    }

    #[test]
    fn looping_wraps_at_the_end_and_the_lead_shows_the_next_loop_early() {
        let tl = timeline(2, FrameRate::FPS_30);
        let frame = FrameRate::FPS_30.frame_duration().flicks();
        let mut picker = FramePicker::default();
        let lead = Duration::from_millis(40);
        // 20 ms before the end, the lead points 20 ms into the next loop: frame 0.
        let before = Time::from_seconds(2) - Time(FLICKS_PER_SECOND / 50);
        let pick = picker.pick(audible(1, 0, before), lead, Rate::FORWARD, tl, true);
        assert_eq!(pick.frame, 0);
        assert!(!reached_end(
            audible(1, 0, Time::from_seconds(2)),
            Rate::FORWARD,
            tl,
            true
        ));
        // The clock wrapped (epoch 1): frames continue from the start.
        let pick = picker.pick(audible(1, 1, Time(frame)), lead, Rate::FORWARD, tl, true);
        assert_eq!(pick.frame, 2);
        // Without loop the lead never shows past the last frame.
        let mut picker = FramePicker::default();
        let pick = picker.pick(audible(1, 0, before), lead, Rate::FORWARD, tl, false);
        assert_eq!(pick.frame, tl.last_frame());
    }

    #[test]
    fn the_end_is_the_duration_forward_and_zero_in_reverse() {
        let tl = timeline(2, FrameRate::FPS_30);
        let at = |t| audible(1, 0, t);
        assert!(!reached_end(
            at(Time::from_seconds(2) - Time(1)),
            Rate::FORWARD,
            tl,
            false
        ));
        assert!(reached_end(
            at(Time::from_seconds(2)),
            Rate::FORWARD,
            tl,
            false
        ));
        assert!(!reached_end(at(Time(1)), Rate::REVERSE, tl, false));
        assert!(reached_end(at(Time::ZERO), Rate::REVERSE, tl, true));
    }

    #[test]
    fn the_refresh_estimate_converges_and_ignores_missed_vsyncs() {
        let mut estimate = RefreshEstimate::default();
        let mut now = Instant::now();
        let vsync = Duration::from_nanos(6_060_606); // 165 Hz
        for n in 0..200 {
            now += if n % 50 == 49 { vsync * 3 } else { vsync };
            estimate.presented(now);
        }
        let error = estimate.interval().abs_diff(vsync);
        assert!(
            error < Duration::from_micros(100),
            "{:?}",
            estimate.interval()
        );
        assert_eq!(estimate.lead(), estimate.interval() * 2);
    }

    #[test]
    fn ticks_are_throttled_to_60_hz_and_the_last_change_always_goes_out() {
        // A 144 Hz vsync for 2 s, polled every vsync while playing.
        let mut throttle = TickThrottle::default();
        let start = Instant::now();
        let vsync = Duration::from_nanos(1_000_000_000 / 144);
        let mut ticks = Vec::new();
        for n in 0..288 {
            let now = start + vsync * n;
            if throttle.poll(now, true) == TickDue::Now {
                throttle.emitted(now);
                ticks.push(now);
            }
        }
        assert!(ticks.len() <= 121 && ticks.len() >= 100, "{}", ticks.len());
        let per_second = ticks
            .iter()
            .filter(|t| t.duration_since(start) < Duration::from_secs(1))
            .count();
        assert!(per_second <= 61, "{per_second}");

        // Paused: nothing until a change; a change right after a tick waits for the interval,
        // then goes out once.
        let now = start + Duration::from_secs(3);
        assert_eq!(throttle.poll(now, false), TickDue::Idle);
        throttle.request();
        assert_eq!(throttle.poll(now, false), TickDue::Now);
        throttle.emitted(now);
        throttle.request();
        throttle.request();
        let soon = now + Duration::from_millis(5);
        match throttle.poll(soon, false) {
            TickDue::After(wait) => assert!(wait <= TICK_INTERVAL && wait > Duration::ZERO),
            other => panic!("{other:?}"),
        }
        let later = now + TICK_INTERVAL;
        assert_eq!(throttle.poll(later, false), TickDue::Now);
        throttle.emitted(later);
        assert_eq!(
            throttle.poll(later + Duration::from_secs(1), false),
            TickDue::Idle
        );
    }
}
