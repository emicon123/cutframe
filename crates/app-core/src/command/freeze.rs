//! Freeze frame: a still of the frame under the playhead (F1.82, design `clip-speed-audio.md`
//! §3.4).

use std::collections::HashMap;

use super::cut::ensure_unlocked;
use super::{Command, CommandError, HistoryLabel, SplitClips};
use crate::media::MediaKind;
use crate::model::{Clip, ClipId, ClipSource, Project, Track, TrackId, TrackKind};
use crate::time::Time;
use crate::timeline::{
    Follow, IdPool, expand_links, follow_main, main_changed, place_clip, repack_main,
    retime_keyframes, shift_clips,
};

/// Inserts a freeze frame of a video clip at the playhead (`insertFreezeFrame`).
///
/// `at` is quantised down to its project frame `t_n`. The clip (`None`: the main-track clip under
/// `t_n`) must contain `t_n` and be a video media clip on a video track. The freeze shows
/// [`Clip::frame_sample_time`] at `t_n`, exactly the frame the preview shows there. When `t_n`
/// lies at least one frame inside both ends, the clip and its linked members split at `t_n`
/// ([`SplitClips`]) and the freeze goes between the halves; in the clip's first frame it goes
/// before the clip, in its last frame after it. The freeze has the clip's name and visual props,
/// no sound and no link, and lasts `duration`.
///
/// - **Main track:** the later main clips ripple by `duration` (repacked when magnetic) and the
///   clips on other tracks follow (§3.7); linked right halves, such as detached audio, shift
///   through the link and leave silence under the freeze.
/// - **Other tracks:** the clips of that track from the insert point on and their linked members
///   shift right by `duration`.
///
/// Fails, changing nothing, with [`CommandError::InvalidValue`] (`duration` not positive),
/// [`CommandError::ClipNotFound`], [`CommandError::NothingToEdit`] (no clip under the playhead),
/// [`CommandError::PropertyNotApplicable`] (not a video media clip), [`CommandError::TrackLocked`],
/// [`CommandError::Overlap`] (a shifted clip would overlap on its track) and
/// [`CommandError::InvalidResult`].
#[derive(Debug)]
pub struct InsertFreezeFrame {
    clip: Option<ClipId>,
    at: Time,
    duration: Time,
    id: ClipId,
    /// Kept so a redo hands out the same right-half ids.
    split: Option<SplitClips>,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl InsertFreezeFrame {
    pub fn new(clip: Option<ClipId>, at: Time, duration: Time) -> Self {
        Self {
            clip,
            at,
            duration,
            id: ClipId::new(),
            split: None,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    /// The id of the freeze clip, fixed at construction so redo recreates it.
    pub fn clip_id(&self) -> ClipId {
        self.id
    }

    /// The clip to freeze and its track index.
    fn source<'a>(
        &self,
        project: &'a Project,
        t_n: Time,
    ) -> Result<(usize, &'a Clip), CommandError> {
        let contains = |c: &Clip| c.start <= t_n && t_n < c.end();
        let (track, clip) = match self.clip {
            Some(id) => {
                let (track, clip) = project.clip(id).ok_or(CommandError::ClipNotFound(id))?;
                (track, Some(clip).filter(|c| contains(c)))
            }
            None => {
                let main = project.main_index().ok_or(CommandError::MainTrack)?;
                (
                    main,
                    project.tracks[main].clips.iter().find(|c| contains(c)),
                )
            }
        };
        let clip = clip.ok_or(CommandError::NothingToEdit)?;
        let video = match clip.source {
            ClipSource::Media { media, .. } => project
                .media_asset(media)
                .is_some_and(|m| m.kind == MediaKind::Video && m.info.video.is_some()),
            _ => false,
        };
        if !video || project.tracks[track].kind != TrackKind::Video {
            return Err(CommandError::PropertyNotApplicable);
        }
        Ok((track, clip))
    }

    fn inserted(&mut self, project: &Project) -> Result<Project, CommandError> {
        if self.duration <= Time::ZERO {
            return Err(CommandError::InvalidValue);
        }
        let rate = project.settings.frame_rate;
        let frame = rate.frame_duration();
        let t_n = Time::from_frames(self.at.to_frames(rate), rate);
        let (track, clip) = self.source(project, t_n)?;
        ensure_unlocked(project, [&clip.id])?;
        let media = clip.source.media().expect("video media clip");
        let source_time = clip
            .frame_sample_time(t_n, frame)
            .expect("the clip contains t_n");

        let inside = clip.start + frame <= t_n && t_n + frame <= clip.end();
        let point = if inside {
            t_n
        } else if t_n - clip.start < frame {
            clip.start
        } else {
            clip.end()
        };
        let mut freeze = Clip {
            id: self.id,
            name: clip.name.clone(),
            start: point,
            duration: self.duration,
            source: ClipSource::Freeze { media, source_time },
            link: None,
            visual: clip.visual.clone(),
            audio: None,
        };
        retime_keyframes(&mut freeze, point - clip.start);

        // The split is the state the ripple and the following start from, so the right halves
        // it creates follow like any other clip.
        let mut split = project.clone();
        if inside {
            let id = clip.id;
            self.split
                .get_or_insert_with(|| SplitClips::new(vec![id], t_n))
                .apply(&mut split)?;
        }
        let mut next = split.clone();
        let main = next.main_index().ok_or(CommandError::MainTrack)?;
        if track == main {
            let clips = &mut next.tracks[main].clips;
            if !next.magnetic {
                for c in clips.iter_mut().filter(|c| c.start >= point) {
                    c.start = c.start + self.duration;
                }
            }
            let index = clips.partition_point(|c| c.start < point);
            clips.insert(index, freeze);
            repack_main(&mut next);
            follow_main(
                &split,
                &mut next,
                Follow {
                    moved: &[],
                    delete_inside: false,
                    track_ids: &mut self.track_ids,
                },
            )?;
        } else {
            let following: Vec<ClipId> = next.tracks[track]
                .clips
                .iter()
                .filter(|c| c.start >= point)
                .map(|c| c.id)
                .collect();
            let moving = expand_links(&next, &following)?;
            ensure_unlocked(&next, &moving)?;
            let deltas: HashMap<ClipId, Time> =
                moving.iter().map(|&id| (id, self.duration)).collect();
            shift_clips(&mut next, &deltas)?;
            place_clip(&mut next.tracks[track], freeze)?;
            if main_changed(&split, &next) {
                follow_main(
                    &split,
                    &mut next,
                    Follow {
                        moved: &moving,
                        delete_inside: false,
                        track_ids: &mut self.track_ids,
                    },
                )?;
            }
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

impl Command for InsertFreezeFrame {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::InsertFreezeFrame
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.inserted(project)?;
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
