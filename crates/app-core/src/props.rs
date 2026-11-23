//! Keyframe-ready clip properties (design `timeline-editing.md` §3.6).
//!
//! Every property that keyframes may animate later is stored as [`Animated<T>`] from schema v3 on,
//! so task 0520 adds keyframe editing and evaluation without a migration. Tier 1 only ever writes
//! `keyframes: []`, and [`Animated::at`] returns the plain value.

use serde::{Deserialize, Serialize};

use crate::time::Time;

/// Highest linear audio gain (+12 dB).
pub const MAX_VOLUME: f32 = 4.0;
/// Largest absolute scale factor on either axis.
pub const MAX_SCALE: f32 = 100.0;

/// A property value that may be animated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Animated<T> {
    /// The value when there are no keyframes.
    pub value: T,
    /// Sorted by `time`, unique times, each within `0..=clip.duration`. Omitted from JSON when
    /// empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe<T>>,
}

impl<T> Animated<T> {
    /// A value without keyframes.
    pub const fn constant(value: T) -> Self {
        Self {
            value,
            keyframes: Vec::new(),
        }
    }
}

impl<T: Copy> Animated<T> {
    /// The value at `local` (offset from the clip start). Pure; preview, export and tests share
    /// it. Tier 1 returns `value`; task 0520 adds interpolation between keyframes.
    pub fn at(&self, local: Time) -> T {
        let _ = local;
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Keyframe<T> {
    /// Offset from the clip's start, in flicks.
    pub time: Time,
    pub value: T,
    /// Interpolation from this keyframe to the next one.
    pub easing: Easing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const ONE: Vec2 = Vec2 { x: 1.0, y: 1.0 };
}

/// Insets as fractions of the display-oriented source.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CropInsets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Flip {
    pub horizontal: bool,
    pub vertical: bool,
}

/// How a clip's picture is sized into the canvas before scale is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    /// Contain: the whole picture is visible.
    #[default]
    Fit,
    /// Cover: the canvas is filled, the picture may be cut.
    Fill,
    /// Both axes scaled to the canvas, ignoring the aspect.
    Stretch,
}

/// Visual properties of a clip that draws (everything on video and text tracks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct VisualProps {
    /// Offset of the clip's centre from the canvas centre, in canvas widths (x) and canvas heights
    /// (y, down). (0, 0) is centred.
    pub position: Animated<Vec2>,
    /// Multiplier of the fitted size; (1, 1) = as placed by `fit`. Non-zero, |s| <= 100.
    pub scale: Animated<Vec2>,
    /// Degrees, clockwise. Unbounded.
    pub rotation: Animated<f32>,
    /// 0..=1.
    pub opacity: Animated<f32>,
    /// Each inset >= 0, left + right < 1, top + bottom < 1. Ignored for colour and text clips.
    pub crop: Animated<CropInsets>,
    pub flip: Flip,
    pub fit: FitMode,
}

impl Default for VisualProps {
    fn default() -> Self {
        Self {
            position: Animated::constant(Vec2::ZERO),
            scale: Animated::constant(Vec2::ONE),
            rotation: Animated::constant(0.0),
            opacity: Animated::constant(1.0),
            crop: Animated::constant(CropInsets::default()),
            flip: Flip::default(),
            fit: FitMode::Fit,
        }
    }
}

/// Audio properties of a clip that makes sound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AudioProps {
    /// Linear gain, 0..=4 (+12 dB); 1 = unchanged.
    pub volume: Animated<f32>,
    /// Lengths of the linear fades, each >= 0, sum <= clip duration. Not animatable.
    pub fade_in: Time,
    pub fade_out: Time,
}

impl Default for AudioProps {
    fn default() -> Self {
        Self {
            volume: Animated::constant(1.0),
            fade_in: Time::ZERO,
            fade_out: Time::ZERO,
        }
    }
}

/// Why a property is out of range ([`VisualProps::check`], [`AudioProps::check`]).
pub type PropertyName = &'static str;

/// Checks the keyframe list shape and every value (the `value` and each keyframe's) with `ok`.
fn check_animated<T>(
    animated: &Animated<T>,
    duration: Time,
    name: PropertyName,
    ok: impl Fn(&T) -> bool,
) -> Result<(), PropertyName> {
    if !ok(&animated.value) {
        return Err(name);
    }
    let mut previous: Option<Time> = None;
    for keyframe in &animated.keyframes {
        let in_clip = keyframe.time >= Time::ZERO && keyframe.time <= duration;
        if !in_clip || previous.is_some_and(|p| p >= keyframe.time) || !ok(&keyframe.value) {
            return Err(name);
        }
        previous = Some(keyframe.time);
    }
    Ok(())
}

impl VisualProps {
    /// Checks the property ranges for a clip of `duration`; returns the name of the first property
    /// out of range.
    pub fn check(&self, duration: Time) -> Result<(), PropertyName> {
        let finite = |v: &Vec2| v.x.is_finite() && v.y.is_finite();
        let scale = |s: f32| s.is_finite() && s != 0.0 && s.abs() <= MAX_SCALE;
        check_animated(&self.position, duration, "position", finite)?;
        check_animated(&self.scale, duration, "scale", |v| scale(v.x) && scale(v.y))?;
        check_animated(&self.rotation, duration, "rotation", |r| r.is_finite())?;
        check_animated(&self.opacity, duration, "opacity", |o| {
            (0.0..=1.0).contains(o)
        })?;
        check_animated(&self.crop, duration, "crop", |c| {
            let insets = [c.left, c.top, c.right, c.bottom];
            insets.iter().all(|i| i.is_finite() && *i >= 0.0)
                && c.left + c.right < 1.0
                && c.top + c.bottom < 1.0
        })
    }
}

impl AudioProps {
    /// Checks the property ranges for a clip of `duration`; returns the name of the first property
    /// out of range.
    pub fn check(&self, duration: Time) -> Result<(), PropertyName> {
        check_animated(&self.volume, duration, "volume", |v| {
            (0.0..=MAX_VOLUME).contains(v)
        })?;
        let fades_ok = self.fade_in >= Time::ZERO
            && self.fade_out >= Time::ZERO
            && self
                .fade_in
                .flicks()
                .checked_add(self.fade_out.flicks())
                .is_some_and(|sum| sum <= duration.flicks());
        if !fades_ok {
            return Err("fades");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_in_range() {
        let duration = Time::from_seconds(1);
        assert_eq!(VisualProps::default().check(duration), Ok(()));
        assert_eq!(AudioProps::default().check(duration), Ok(()));
    }

    #[test]
    fn animated_at_returns_the_value_in_tier_1() {
        let mut opacity = Animated::constant(0.5_f32);
        opacity.keyframes.push(Keyframe {
            time: Time::ZERO,
            value: 1.0,
            easing: Easing::Hold,
        });
        assert_eq!(opacity.at(Time::from_seconds(3)), 0.5);
    }

    #[test]
    fn empty_keyframes_are_omitted_and_written_ones_load() {
        let json = serde_json::to_value(Animated::constant(0.25_f32)).unwrap();
        assert_eq!(json, serde_json::json!({ "value": 0.25 }));
        let loaded: Animated<Vec2> = serde_json::from_value(serde_json::json!({
            "value": { "x": 0.0, "y": 0.0 },
            "keyframes": [
                { "time": 0, "value": { "x": 0.1, "y": 0.0 }, "easing": "easeInOut" },
                { "time": 705600000, "value": { "x": 0.2, "y": 0.0 }, "easing": "hold" }
            ]
        }))
        .unwrap();
        assert_eq!(loaded.keyframes.len(), 2);
        assert_eq!(loaded.keyframes[0].easing, Easing::EaseInOut);
    }

    #[test]
    fn ranges_are_checked() {
        let d = Time::from_seconds(1);
        let with = |edit: fn(&mut VisualProps)| {
            let mut props = VisualProps::default();
            edit(&mut props);
            props.check(d)
        };
        assert_eq!(with(|p| p.opacity.value = 1.5), Err("opacity"));
        assert_eq!(with(|p| p.opacity.value = f32::NAN), Err("opacity"));
        assert_eq!(with(|p| p.scale.value.x = 0.0), Err("scale"));
        assert_eq!(with(|p| p.scale.value.y = -101.0), Err("scale"));
        assert_eq!(with(|p| p.scale.value.y = -100.0), Ok(()));
        assert_eq!(with(|p| p.rotation.value = f32::INFINITY), Err("rotation"));
        assert_eq!(with(|p| p.rotation.value = 7200.0), Ok(()));
        assert_eq!(with(|p| p.position.value.x = f32::NAN), Err("position"));
        assert_eq!(
            with(|p| {
                p.crop.value.left = 0.5;
                p.crop.value.right = 0.5;
            }),
            Err("crop")
        );
        assert_eq!(with(|p| p.crop.value.top = -0.1), Err("crop"));
        assert_eq!(with(|p| p.crop.value.top = 0.9), Ok(()));

        let mut audio = AudioProps::default();
        audio.volume.value = 4.5;
        assert_eq!(audio.check(d), Err("volume"));
        let audio = AudioProps {
            fade_in: Time(d.flicks() / 2),
            fade_out: Time(d.flicks() / 2 + 1),
            ..AudioProps::default()
        };
        assert_eq!(audio.check(d), Err("fades"));
    }

    #[test]
    fn keyframes_must_be_sorted_unique_and_inside_the_clip() {
        let d = Time::from_seconds(1);
        let key = |t: i64| Keyframe {
            time: Time(t),
            value: 0.5_f32,
            easing: Easing::Linear,
        };
        let mut props = VisualProps::default();
        props.opacity.keyframes = vec![key(0), key(d.flicks())];
        assert_eq!(props.check(d), Ok(()));
        props.opacity.keyframes = vec![key(5), key(5)];
        assert_eq!(props.check(d), Err("opacity"));
        props.opacity.keyframes = vec![key(d.flicks() + 1)];
        assert_eq!(props.check(d), Err("opacity"));
        props.opacity.keyframes = vec![Keyframe {
            value: 2.0,
            ..key(0)
        }];
        assert_eq!(props.check(d), Err("opacity"));
    }
}
