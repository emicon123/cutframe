//! Integer time. All positions and durations in the model are [`Time`] values in flicks.
//!
//! A flick is 1/705,600,000 of a second. It divides evenly into every common video frame rate
//! (including NTSC rates such as 30000/1001) and audio sample rate, so frame-accurate edits never
//! accumulate floating-point error.

use std::ops::{Add, Neg, Sub};

use serde::{Deserialize, Serialize};

/// Flicks in one second.
pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// Largest time that crosses IPC exactly: TypeScript receives [`Time`] as a `number`, which is exact
/// within ±2^53 (about 147 days of timeline). Commands and project validation reject later times.
pub const MAX_IPC_FLICKS: i64 = 1 << 53;

/// A point in time or a duration, in flicks.
///
/// Exposed to TypeScript as `number`: values stay below 2^53 for timelines shorter than ~147 days.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct Time(pub i64);

impl Time {
    pub const ZERO: Time = Time(0);

    pub const fn from_flicks(flicks: i64) -> Self {
        Self(flicks)
    }

    pub const fn flicks(self) -> i64 {
        self.0
    }

    pub const fn from_seconds(seconds: i64) -> Self {
        Self(seconds * FLICKS_PER_SECOND)
    }

    /// The start time of frame `frame` at `rate`. Exact for every rate whose period is a whole
    /// number of flicks (all rates accepted by [`FrameRate::new`]).
    pub const fn from_frames(frame: i64, rate: FrameRate) -> Self {
        Self(frame * rate.frame_duration().0)
    }

    /// Index of the frame that contains this time (floor).
    pub const fn to_frames(self, rate: FrameRate) -> i64 {
        self.0.div_euclid(rate.frame_duration().0)
    }

    /// For display and shaders only. Never feed the result back into the model.
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }
}

impl Add for Time {
    type Output = Time;
    fn add(self, rhs: Time) -> Time {
        Time(self.0 + rhs.0)
    }
}

impl Sub for Time {
    type Output = Time;
    fn sub(self, rhs: Time) -> Time {
        Time(self.0 - rhs.0)
    }
}

impl Neg for Time {
    type Output = Time;
    fn neg(self) -> Time {
        Time(-self.0)
    }
}

/// A frame rate as a rational number of frames per second, e.g. `30000/1001` for 29.97 fps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(try_from = "FrameRateRepr", into = "FrameRateRepr")]
pub struct FrameRate {
    num: u32,
    den: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameRateError {
    #[error("frame rate {num}/{den} is not positive")]
    NotPositive { num: u32, den: u32 },
    #[error("frame rate {num}/{den} has a frame period that is not a whole number of flicks")]
    NotFlickAligned { num: u32, den: u32 },
}

impl FrameRate {
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };
    pub const NTSC_30: FrameRate = FrameRate {
        num: 30000,
        den: 1001,
    };

    pub fn new(num: u32, den: u32) -> Result<Self, FrameRateError> {
        if num == 0 || den == 0 {
            return Err(FrameRateError::NotPositive { num, den });
        }
        if (FLICKS_PER_SECOND * i64::from(den)) % i64::from(num) != 0 {
            return Err(FrameRateError::NotFlickAligned { num, den });
        }
        Ok(Self { num, den })
    }

    pub const fn num(self) -> u32 {
        self.num
    }

    pub const fn den(self) -> u32 {
        self.den
    }

    /// Duration of one frame. Exact by construction.
    pub const fn frame_duration(self) -> Time {
        Time(FLICKS_PER_SECOND * self.den as i64 / self.num as i64)
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
struct FrameRateRepr {
    num: u32,
    den: u32,
}

impl TryFrom<FrameRateRepr> for FrameRate {
    type Error = FrameRateError;
    fn try_from(r: FrameRateRepr) -> Result<Self, Self::Error> {
        FrameRate::new(r.num, r.den)
    }
}

impl From<FrameRate> for FrameRateRepr {
    fn from(r: FrameRate) -> Self {
        Self {
            num: r.num,
            den: r.den,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_frame_rates_are_flick_aligned() {
        for (num, den) in [
            (24, 1),
            (24000, 1001),
            (25, 1),
            (30, 1),
            (30000, 1001),
            (48, 1),
            (50, 1),
            (60, 1),
            (60000, 1001),
            (120, 1),
        ] {
            assert!(FrameRate::new(num, den).is_ok(), "{num}/{den}");
        }
    }

    #[test]
    fn rejects_invalid_rates() {
        assert_eq!(
            FrameRate::new(0, 1),
            Err(FrameRateError::NotPositive { num: 0, den: 1 })
        );
        assert!(matches!(
            FrameRate::new(11, 1),
            Err(FrameRateError::NotFlickAligned { .. })
        ));
    }

    #[test]
    fn frames_round_trip_exactly() {
        let rate = FrameRate::NTSC_30;
        for frame in [0, 1, 29, 30, 1799, 107_892] {
            assert_eq!(Time::from_frames(frame, rate).to_frames(rate), frame);
        }
    }

    #[test]
    fn ntsc_frames_have_no_drift() {
        // 30000 frames at 30000/1001 fps last exactly 1001 seconds.
        assert_eq!(
            Time::from_frames(30000, FrameRate::NTSC_30),
            Time::from_seconds(1001)
        );
    }

    #[test]
    fn negative_times_floor_to_previous_frame() {
        let rate = FrameRate::FPS_30;
        assert_eq!(Time(-1).to_frames(rate), -1);
    }
}
