//! Speed and reverse of media clips (design `clip-speed-audio.md` §3.4).
//!
//! Both work on a copy of the project and swap the tracks in only when the result is valid, so a
//! failed apply changes nothing.

use super::cut::ensure_unlocked;
use super::{Command, CommandError, HistoryLabel};
use crate::media::MediaKind;
use crate::model::{ClipId, ClipSource, Project, Track, TrackId};
use crate::time::{Speed, Time};
use crate::timeline::{
    Follow, IdPool, expand_links, follow_main, main_changed, repack_main, retime_keyframes,
};

/// The video and audio media clips among `clips` and their linked members, in track order then
/// clip order. Fails with [`CommandError::NothingToEdit`] for no clips,
/// [`CommandError::ClipNotFound`], [`CommandError::PropertyNotApplicable`] when none is a video
/// or audio media clip (images, freeze, colour and text clips are skipped) and
/// [`CommandError::TrackLocked`] when one of them is on a locked track.
fn targets(project: &Project, clips: &[ClipId]) -> Result<Vec<ClipId>, CommandError> {
    if clips.is_empty() {
        return Err(CommandError::NothingToEdit);
    }
    let targets: Vec<ClipId> = expand_links(project, clips)?
        .into_iter()
        .filter(|&id| {
            project.clip(id).is_some_and(|(_, clip)| match clip.source {
                ClipSource::Media { media, .. } => project
                    .media_asset(media)
                    .is_some_and(|m| m.kind != MediaKind::Image),
                _ => false,
            })
        })
        .collect();
    if targets.is_empty() {
        return Err(CommandError::PropertyNotApplicable);
    }
    ensure_unlocked(project, &targets)?;
    Ok(targets)
}

fn clip_mut(project: &mut Project, id: ClipId) -> &mut crate::model::Clip {
    project
        .tracks
        .iter_mut()
        .flat_map(|t| t.clips.iter_mut())
        .find(|c| c.id == id)
        .expect("target exists")
}

/// Whether two clips of `track` overlap.
fn overlaps(track: &Track) -> bool {
    track.clips.windows(2).any(|w| w[0].end() > w[1].start)
}

/// Sets the speed of clips and their linked members (`setClipSpeed`, an ADR-0006 gesture for the
/// slider; design `clip-speed-audio.md` §3.4).
///
/// Targets the video and audio media clips among them. Per target the start (unless the main
/// track moves it) and the used source range stay and the duration becomes
/// `floor(span x 100 / speed)`; a reversed clip keeps the top
/// of its range, so its first frame stays. Fades shrink to fit. On the magnetic main track the
/// later clips ripple, and the clips on other tracks follow the main track (§3.7), so B-roll stays
/// over the same content moment of a sped-up clip.
///
/// Fails, changing nothing, with the errors of the target rules (no target:
/// [`CommandError::PropertyNotApplicable`]), [`CommandError::InvalidValue`] when a target would
/// get shorter than one project frame, [`CommandError::Overlap`] when a target off the magnetic
/// main track would overlap a clip on its track, and [`CommandError::InvalidResult`]. A speed
/// every target already has changes nothing ([`SetClipSpeed::changes`]).
#[derive(Debug)]
pub struct SetClipSpeed {
    clips: Vec<ClipId>,
    speed: Speed,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl SetClipSpeed {
    pub fn new(clips: Vec<ClipId>, speed: Speed) -> Self {
        Self {
            clips,
            speed,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    /// Whether applying to `project` would change anything. Fails with the target errors.
    pub fn changes(&self, project: &Project) -> Result<bool, CommandError> {
        let targets = targets(project, &self.clips)?;
        Ok(targets.iter().any(|&id| {
            project
                .clip(id)
                .is_some_and(|(_, c)| c.speed() != self.speed)
        }))
    }

    fn retimed(&mut self, project: &Project) -> Result<Project, CommandError> {
        let targets = targets(project, &self.clips)?;
        let frame = project.settings.frame_rate.frame_duration();
        let mut next = project.clone();
        for &id in &targets {
            let clip = clip_mut(&mut next, id);
            let ClipSource::Media {
                source_in,
                speed,
                reverse,
                ..
            } = &mut clip.source
            else {
                continue;
            };
            if *speed == self.speed {
                continue;
            }
            let span = speed.source_ceil(clip.duration);
            let duration = self.speed.timeline_floor(span);
            if duration <= Time::ZERO || (duration < frame && duration < clip.duration) {
                return Err(CommandError::InvalidValue);
            }
            if *reverse {
                *source_in = *source_in + span - self.speed.source_ceil(duration);
            }
            *speed = self.speed;
            clip.duration = duration;
            retime_keyframes(clip, Time::ZERO);
        }
        let main = next.main_index().ok_or(CommandError::MainTrack)?;
        if !next.magnetic && overlaps(&next.tracks[main]) {
            return Err(CommandError::Overlap);
        }
        repack_main(&mut next);
        if main_changed(project, &next) {
            // Targets follow too: the sound of a later main clip moves with it, and a sped-up
            // overlay stays over its moment of the main track.
            follow_main(
                project,
                &mut next,
                Follow {
                    moved: &[],
                    delete_inside: false,
                    track_ids: &mut self.track_ids,
                },
            )?;
        }
        if next.tracks.iter().any(overlaps) {
            return Err(CommandError::Overlap);
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

impl Command for SetClipSpeed {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SetClipSpeed
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.retimed(project)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

/// Turns reverse playback of clips and their linked members on or off (`setClipReverse`).
///
/// Targets the video and audio media clips among them, with the rules and errors of
/// [`SetClipSpeed`]. Start, duration and the used source range stay; nothing ripples or follows.
/// A value every target already has changes nothing ([`SetClipReverse::changes`]).
#[derive(Debug)]
pub struct SetClipReverse {
    clips: Vec<ClipId>,
    reverse: bool,
    previous: Option<Vec<Track>>,
}

impl SetClipReverse {
    pub fn new(clips: Vec<ClipId>, reverse: bool) -> Self {
        Self {
            clips,
            reverse,
            previous: None,
        }
    }

    /// Whether applying to `project` would change anything. Fails with the target errors.
    pub fn changes(&self, project: &Project) -> Result<bool, CommandError> {
        let targets = targets(project, &self.clips)?;
        Ok(targets.iter().any(|&id| {
            project
                .clip(id)
                .is_some_and(|(_, c)| c.reversed() != self.reverse)
        }))
    }
}

impl Command for SetClipReverse {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SetClipReverse
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let targets = targets(project, &self.clips)?;
        let mut next = project.clone();
        for id in targets {
            if let ClipSource::Media { reverse, .. } = &mut clip_mut(&mut next, id).source {
                *reverse = self.reverse;
            }
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

#[cfg(test)]
mod tests;
