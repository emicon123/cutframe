//! Setting clip properties from the inspector (design `timeline-editing.md` §4.10,
//! `clip-speed-audio.md` §3.4).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{Command, CommandError, HistoryLabel};
use crate::model::{Clip, ClipId, Project};
use crate::props::{AudioProps, CropInsets, FitMode, Flip, Vec2, VisualProps};
use crate::time::Time;

/// One property and its new value. Animatable properties set `Animated.value`; tier 1 writes no
/// keyframes (task 0520 defines "set at playhead" when keyframes exist).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "property", content = "value", rename_all = "camelCase")]
pub enum PropertyValue {
    /// Offset of the clip's centre from the canvas centre, in canvas widths and heights.
    Position(Vec2),
    /// Non-zero, |s| <= 100 on each axis.
    Scale(Vec2),
    /// Degrees, clockwise, finite.
    Rotation(f32),
    /// 0..=1.
    Opacity(f32),
    /// Each inset >= 0, left + right < 1, top + bottom < 1.
    Crop(CropInsets),
    Flip(Flip),
    Fit(FitMode),
    /// Linear gain, 0..=4.
    Volume(f32),
    /// Fade-in length, >= 0; clamped per clip to its duration minus its fade-out.
    FadeIn(Time),
    /// Fade-out length, >= 0; clamped per clip to its duration minus its fade-in.
    FadeOut(Time),
    /// Keep the pitch at speeds other than 1x.
    PreservePitch(bool),
}

/// Names a property, e.g. for resetting it (task 0350).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PropertyKey {
    Position,
    Scale,
    Rotation,
    Opacity,
    Crop,
    Flip,
    Fit,
    Volume,
    FadeIn,
    FadeOut,
    PreservePitch,
}

impl PropertyValue {
    pub fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::Position(_) => PropertyKey::Position,
            PropertyValue::Scale(_) => PropertyKey::Scale,
            PropertyValue::Rotation(_) => PropertyKey::Rotation,
            PropertyValue::Opacity(_) => PropertyKey::Opacity,
            PropertyValue::Crop(_) => PropertyKey::Crop,
            PropertyValue::Flip(_) => PropertyKey::Flip,
            PropertyValue::Fit(_) => PropertyKey::Fit,
            PropertyValue::Volume(_) => PropertyKey::Volume,
            PropertyValue::FadeIn(_) => PropertyKey::FadeIn,
            PropertyValue::FadeOut(_) => PropertyKey::FadeOut,
            PropertyValue::PreservePitch(_) => PropertyKey::PreservePitch,
        }
    }

    /// `visual` with this value set; `None` if it is not a visual property.
    fn set_visual(self, visual: &VisualProps) -> Option<VisualProps> {
        let mut v = visual.clone();
        match self {
            PropertyValue::Position(p) => v.position.value = p,
            PropertyValue::Scale(s) => v.scale.value = s,
            PropertyValue::Rotation(r) => v.rotation.value = r,
            PropertyValue::Opacity(o) => v.opacity.value = o,
            PropertyValue::Crop(c) => v.crop.value = c,
            PropertyValue::Flip(f) => v.flip = f,
            PropertyValue::Fit(f) => v.fit = f,
            _ => return None,
        }
        Some(v)
    }

    /// `audio` of a clip of `duration` with this value set (fades clamped to the room the other
    /// fade leaves); `None` if it is not an audio property.
    fn set_audio(self, audio: &AudioProps, duration: Time) -> Option<AudioProps> {
        let mut a = audio.clone();
        match self {
            PropertyValue::Volume(v) => a.volume.value = v,
            PropertyValue::FadeIn(f) => a.fade_in = f.min(duration - a.fade_out),
            PropertyValue::FadeOut(f) => a.fade_out = f.min(duration - a.fade_in),
            PropertyValue::PreservePitch(p) => a.preserve_pitch = p,
            _ => return None,
        }
        Some(a)
    }
}

/// Sets one property on every listed clip that has it (`setClipProperty`, an ADR-0006 gesture
/// for sliders; design `timeline-editing.md` §4.10).
///
/// Visual properties apply to clips with `visual` (video and text tracks), audio properties to
/// clips with `audio`; the others are skipped. No link expansion: props are per clip. Fades are
/// clamped per clip to `0..=duration - the other fade`, so a drag stops at the limit.
///
/// Fails, changing nothing, with [`CommandError::NothingToEdit`] (no clips),
/// [`CommandError::ClipNotFound`], [`CommandError::PropertyNotApplicable`] (no listed clip has the
/// property), [`CommandError::TrackLocked`] (a clip that has it is on a locked track) and
/// [`CommandError::InvalidValue`] (a non-finite or out-of-range value, a negative fade). A value
/// every clip already has changes nothing ([`SetClipProperty::changes`]).
#[derive(Debug)]
pub struct SetClipProperty {
    clips: Vec<ClipId>,
    value: PropertyValue,
    /// The props before `apply` of every changed clip.
    previous: Vec<Props>,
}

/// A clip's props, by clip id.
#[derive(Debug)]
struct Props {
    clip: ClipId,
    visual: Option<VisualProps>,
    audio: Option<AudioProps>,
}

impl SetClipProperty {
    pub fn new(clips: Vec<ClipId>, value: PropertyValue) -> Self {
        Self {
            clips,
            value,
            previous: Vec::new(),
        }
    }

    /// Whether applying to `project` would change anything. Fails as `apply` would.
    pub fn changes(&self, project: &Project) -> Result<bool, CommandError> {
        Ok(!self.updates(project)?.is_empty())
    }

    /// The new props of every clip whose props change.
    fn updates(&self, project: &Project) -> Result<Vec<Props>, CommandError> {
        if self.clips.is_empty() {
            return Err(CommandError::NothingToEdit);
        }
        if matches!(self.value, PropertyValue::FadeIn(f) | PropertyValue::FadeOut(f) if f < Time::ZERO)
        {
            return Err(CommandError::InvalidValue);
        }
        let mut seen = HashSet::new();
        let mut applicable = false;
        let mut updates = Vec::new();
        for &id in &self.clips {
            if !seen.insert(id) {
                continue;
            }
            let (track, clip) = project.clip(id).ok_or(CommandError::ClipNotFound(id))?;
            let Some(update) = self.updated(clip)? else {
                continue;
            };
            applicable = true;
            let track = &project.tracks[track];
            if track.locked {
                return Err(CommandError::TrackLocked(track.id));
            }
            if update.visual != clip.visual || update.audio != clip.audio {
                updates.push(update);
            }
        }
        if !applicable {
            return Err(CommandError::PropertyNotApplicable);
        }
        Ok(updates)
    }

    /// `clip`'s props with the value set and range-checked; `None` if it lacks the property.
    fn updated(&self, clip: &Clip) -> Result<Option<Props>, CommandError> {
        let mut props = Props {
            clip: clip.id,
            visual: clip.visual.clone(),
            audio: clip.audio.clone(),
        };
        if let Some(visual) = clip.visual.as_ref().and_then(|v| self.value.set_visual(v)) {
            visual
                .check(clip.duration)
                .map_err(|_| CommandError::InvalidValue)?;
            props.visual = Some(visual);
        } else if let Some(audio) = clip
            .audio
            .as_ref()
            .and_then(|a| self.value.set_audio(a, clip.duration))
        {
            audio
                .check(clip.duration)
                .map_err(|_| CommandError::InvalidValue)?;
            props.audio = Some(audio);
        } else {
            return Ok(None);
        }
        Ok(Some(props))
    }
}

impl Command for SetClipProperty {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SetClipProperty
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let updates = self.updates(project)?;
        self.previous.clear();
        for update in updates {
            let clip = clip_mut(project, update.clip);
            self.previous.push(Props {
                clip: update.clip,
                visual: std::mem::replace(&mut clip.visual, update.visual),
                audio: std::mem::replace(&mut clip.audio, update.audio),
            });
        }
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        for previous in self.previous.drain(..) {
            let clip = clip_mut(project, previous.clip);
            clip.visual = previous.visual;
            clip.audio = previous.audio;
        }
    }
}

fn clip_mut(project: &mut Project, id: ClipId) -> &mut Clip {
    project
        .tracks
        .iter_mut()
        .flat_map(|t| t.clips.iter_mut())
        .find(|c| c.id == id)
        .expect("listed clip exists")
}

#[cfg(test)]
mod tests;
