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

    /// Converts an FFmpeg-style timestamp, `ticks` units of `time_base` seconds, to flicks, rounded
    /// to the nearest flick (halves away from zero) and saturated at the `i64` range. With
    /// [`Time::to_ticks`] this is the only place where media timestamps become model time.
    pub fn from_ticks(ticks: i64, time_base: Rational) -> Self {
        let numerator =
            i128::from(ticks) * i128::from(time_base.num) * i128::from(FLICKS_PER_SECOND);
        Self(saturate(div_round(numerator, i128::from(time_base.den))))
    }

    /// The timestamp in units of `time_base` seconds that is nearest to this time (halves away
    /// from zero), saturated at the `i64` range.
    pub fn to_ticks(self, time_base: Rational) -> i64 {
        let numerator = i128::from(self.0) * i128::from(time_base.den);
        let denominator = i128::from(time_base.num) * i128::from(FLICKS_PER_SECOND);
        saturate(div_round(numerator, denominator))
    }

    /// For display and shaders only. Never feed the result back into the model.
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }
}

/// `numerator / denominator` rounded to the nearest integer, halves away from zero.
/// `denominator` is positive.
fn div_round(numerator: i128, denominator: i128) -> i128 {
    let half = denominator / 2;
    if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        (numerator - half) / denominator
    }
}

fn saturate(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// An exact positive ratio such as a source frame rate (30000/1001), a pixel aspect (4/3) or an
/// FFmpeg time base (1/90000). Unlike [`FrameRate`] it need not be flick-aligned: source rates such
/// as 2997/100 are valid. Always reduced, with `num > 0` and `den > 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(try_from = "RationalRepr", into = "RationalRepr")]
pub struct Rational {
    num: u32,
    den: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("ratio {num}/{den} is not positive")]
pub struct RationalError {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// `num/den`, reduced. Fails unless both are positive and the reduced terms fit in `u32`.
    pub fn new(num: i64, den: i64) -> Result<Self, RationalError> {
        if num <= 0 || den <= 0 {
            return Err(RationalError { num, den });
        }
        let gcd = gcd(num.unsigned_abs(), den.unsigned_abs());
        match (
            u32::try_from(num.unsigned_abs() / gcd),
            u32::try_from(den.unsigned_abs() / gcd),
        ) {
            (Ok(n), Ok(d)) => Ok(Self { num: n, den: d }),
            _ => Err(RationalError { num, den }),
        }
    }

    pub const fn num(self) -> u32 {
        self.num
    }

    pub const fn den(self) -> u32 {
        self.den
    }

    /// For display only.
    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
struct RationalRepr {
    num: u32,
    den: u32,
}

impl TryFrom<RationalRepr> for Rational {
    type Error = RationalError;
    fn try_from(r: RationalRepr) -> Result<Self, Self::Error> {
        let rational = Rational::new(i64::from(r.num), i64::from(r.den))?;
        // The file stores reduced ratios; a non-reduced one was not written by this app.
        if rational.num != r.num {
            return Err(RationalError {
                num: r.num.into(),
                den: r.den.into(),
            });
        }
        Ok(rational)
    }
}

impl From<Rational> for RationalRepr {
    fn from(r: Rational) -> Self {
        Self {
            num: r.num,
            den: r.den,
        }
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

    fn tb(num: i64, den: i64) -> Rational {
        Rational::new(num, den).unwrap()
    }

    #[test]
    fn rationals_are_positive_and_reduced() {
        assert_eq!(tb(60000, 2002), tb(30000, 1001));
        assert_eq!(tb(8, 6).num(), 4);
        assert!(Rational::new(0, 1).is_err());
        assert!(Rational::new(1, -1).is_err());
        assert!(Rational::new(i64::MAX, 1).is_err());
        // The file stores reduced ratios only.
        assert!(serde_json::from_str::<Rational>(r#"{"num":2,"den":4}"#).is_err());
        assert!(serde_json::from_str::<Rational>(r#"{"num":0,"den":4}"#).is_err());
        assert_eq!(
            serde_json::from_str::<Rational>(r#"{"num":4,"den":3}"#).unwrap(),
            tb(4, 3)
        );
    }

    #[test]
    fn ticks_convert_exactly_for_common_time_bases() {
        assert_eq!(
            Time::from_ticks(90_000, tb(1, 90_000)),
            Time::from_seconds(1)
        );
        assert_eq!(Time::from_ticks(600, tb(1, 600)), Time::from_seconds(1));
        assert_eq!(Time::from_ticks(1, tb(1, 44_100)), Time(16_000));
        assert_eq!(Time::from_ticks(1, tb(1, 48_000)), Time(14_700));
        // One frame in a 1001/30000 time base is one NTSC frame.
        assert_eq!(
            Time::from_ticks(1, tb(1001, 30_000)),
            FrameRate::NTSC_30.frame_duration()
        );
        assert_eq!(Time::from_seconds(1).to_ticks(tb(1, 90_000)), 90_000);
        assert_eq!(
            FrameRate::NTSC_30
                .frame_duration()
                .to_ticks(tb(1001, 30_000)),
            1
        );
    }

    #[test]
    fn ticks_round_to_nearest() {
        // 1 µs = 705.6 flicks.
        assert_eq!(Time::from_ticks(1, tb(1, 1_000_000)), Time(706));
        assert_eq!(Time::from_ticks(-1, tb(1, 1_000_000)), Time(-706));
        // 1000 flicks = 1.417... µs.
        assert_eq!(Time(1000).to_ticks(tb(1, 1_000_000)), 1);
        assert_eq!(Time(-1100).to_ticks(tb(1, 1_000_000)), -2);
    }

    #[test]
    fn ticks_saturate_instead_of_overflowing() {
        assert_eq!(Time::from_ticks(i64::MAX, tb(1, 1)), Time(i64::MAX));
        assert_eq!(Time::from_ticks(i64::MIN, tb(1, 1)), Time(i64::MIN));
        let limit = Time(1 << 53);
        for base in [
            tb(1, 90_000),
            tb(1001, 30_000),
            tb(1, 44_100),
            tb(1, 1_000_000),
        ] {
            let ticks = limit.to_ticks(base);
            assert!((Time::from_ticks(ticks, base) - limit).0.abs() <= Time::from_ticks(1, base).0);
            let ticks = (-limit).to_ticks(base);
            assert!((Time::from_ticks(ticks, base) + limit).0.abs() <= Time::from_ticks(1, base).0);
        }
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn time_base() -> impl Strategy<Value = Rational> {
            (1i64..=100_000, 1i64..=1_000_000).prop_map(|(n, d)| Rational::new(n, d).unwrap())
        }

        proptest! {
            /// time -> ticks -> time lands within one tick of the start.
            #[test]
            fn round_trip_stays_within_one_tick(
                flicks in -(1i64 << 53)..(1i64 << 53),
                base in time_base(),
            ) {
                let time = Time(flicks);
                let back = Time::from_ticks(time.to_ticks(base), base);
                let tick = Time::from_ticks(1, base).0.max(1);
                prop_assert!((back - time).0.abs() <= tick, "{time:?} -> {back:?}");
            }

            /// ticks -> time -> ticks is exact for media time bases (1/1000 s and finer, one tick
            /// at least two flicks) over days of media.
            #[test]
            fn ticks_survive_a_round_trip(
                ticks in -(1i64 << 36)..(1i64 << 36),
                num in 1i64..=1001,
                den in 1000i64..=1_000_000,
            ) {
                let base = Rational::new(num, den).unwrap();
                prop_assert_eq!(Time::from_ticks(ticks, base).to_ticks(base), ticks);
            }
        }
    }

    #[test]
    fn negative_times_floor_to_previous_frame() {
        let rate = FrameRate::FPS_30;
        assert_eq!(Time(-1).to_frames(rate), -1);
    }
}
