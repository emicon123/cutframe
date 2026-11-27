//! Links between clips: link, unlink and detach audio (design `timeline-editing.md` §3.5,
//! §4.12, §4.13).
//!
//! Move, trim, split and delete already expand a clip to its link group; these commands create
//! and remove the groups. Each works on a copy of the project and swaps the tracks in only when
//! the result is valid, so a failed apply changes nothing.

use std::collections::HashSet;

use super::{Command, CommandError, HistoryLabel};
use crate::model::{Clip, ClipId, LinkId, Project, Track, TrackId, TrackKind};
use crate::timeline::{IdPool, is_free, normalize_links, place_clip, section_index};

/// The listed clips, each once, in track order then clip order, with their track index. Fails
/// with [`CommandError::ClipNotFound`] for an unknown id and [`CommandError::TrackLocked`] for a
/// clip on a locked track.
fn unlocked_clips(project: &Project, ids: &[ClipId]) -> Result<Vec<(usize, ClipId)>, CommandError> {
    for &id in ids {
        let (track, _) = project.clip(id).ok_or(CommandError::ClipNotFound(id))?;
        let track = &project.tracks[track];
        if track.locked {
            return Err(CommandError::TrackLocked(track.id));
        }
    }
    Ok(project
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(i, t)| t.clips.iter().map(move |c| (i, c.id)))
        .filter(|(_, id)| ids.contains(id))
        .collect())
}

fn clip_mut(project: &mut Project, track: usize, id: ClipId) -> &mut Clip {
    project.tracks[track]
        .clips
        .iter_mut()
        .find(|c| c.id == id)
        .expect("clip is on its track")
}

/// Links two or more clips on pairwise different tracks into one new group (`linkClips`, the
/// link toggle's "on"). Their previous links are replaced; a previous group left with one clip
/// loses its link. Fails with [`CommandError::InvalidLink`] for fewer than two clips or two clips
/// on one track, and with [`CommandError::TrackLocked`] for a clip on a locked track.
#[derive(Debug)]
pub struct LinkClips {
    clips: Vec<ClipId>,
    link: LinkId,
    previous: Option<Vec<Track>>,
}

impl LinkClips {
    pub fn new(clips: Vec<ClipId>) -> Self {
        Self {
            clips,
            link: LinkId::new(),
            previous: None,
        }
    }
}

impl Command for LinkClips {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::LinkClips
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let clips = unlocked_clips(project, &self.clips)?;
        let tracks: HashSet<usize> = clips.iter().map(|(t, _)| *t).collect();
        if clips.len() < 2 || tracks.len() != clips.len() {
            return Err(CommandError::InvalidLink);
        }
        let mut next = project.clone();
        for &(track, id) in &clips {
            clip_mut(&mut next, track, id).link = Some(self.link);
        }
        normalize_links(&mut next);
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

/// Clears the link of the listed clips (`unlinkClips`, the link toggle's "off"). Only the listed
/// clips leave their groups; a group left with one clip loses its link too. Fails with
/// [`CommandError::NothingToEdit`] when none of the clips is linked and with
/// [`CommandError::TrackLocked`] for a clip on a locked track.
#[derive(Debug)]
pub struct UnlinkClips {
    clips: Vec<ClipId>,
    previous: Option<Vec<Track>>,
}

impl UnlinkClips {
    pub fn new(clips: Vec<ClipId>) -> Self {
        Self {
            clips,
            previous: None,
        }
    }
}

impl Command for UnlinkClips {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::UnlinkClips
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let clips = unlocked_clips(project, &self.clips)?;
        let mut next = project.clone();
        let mut changed = false;
        for &(track, id) in &clips {
            changed |= clip_mut(&mut next, track, id).link.take().is_some();
        }
        if !changed {
            return Err(CommandError::NothingToEdit);
        }
        normalize_links(&mut next);
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

/// Moves the sound of video-track media clips onto audio tracks (`detachAudio`, design §3.5).
///
/// For each listed video-track clip with `audio` props, a new audio clip with the same source
/// (media, `source_in`, speed, direction), `start`, `duration` and name takes over the `audio`
/// props, and the video clip's
/// `audio` becomes `None`. The two are linked: the new clip joins the video clip's group, or both
/// get a new link. The new clip goes on the first unlocked audio track where its range is free
/// and no member of its group lies, else on a new audio track at the top of the audio section.
/// Other clips are skipped. Fails with [`CommandError::NothingToEdit`] when no clip was detached
/// and with [`CommandError::TrackLocked`] for a listed clip on a locked track.
#[derive(Debug)]
pub struct DetachAudio {
    clips: Vec<ClipId>,
    clip_ids: IdPool<ClipId>,
    links: IdPool<LinkId>,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl DetachAudio {
    pub fn new(clips: Vec<ClipId>) -> Self {
        Self {
            clips,
            clip_ids: IdPool::default(),
            links: IdPool::default(),
            track_ids: IdPool::default(),
            previous: None,
        }
    }
}

impl Command for DetachAudio {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::DetachAudio
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.clip_ids.rewind();
        self.links.rewind();
        self.track_ids.rewind();
        let clips = unlocked_clips(project, &self.clips)?;
        let mut next = project.clone();
        let mut detached = false;
        for (_, id) in clips {
            // Track indices shift when a new audio track is created, so look the clip up again.
            let (track, clip) = next.clip(id).expect("listed clip");
            if next.tracks[track].kind != TrackKind::Video
                || clip.source.media().is_none()
                || clip.audio.is_none()
            {
                continue;
            }
            let link = match clip.link {
                Some(link) => link,
                None => self.links.take(),
            };
            let video = clip_mut(&mut next, track, id);
            video.link = Some(link);
            let sound = Clip {
                id: self.clip_ids.take(),
                visual: None,
                audio: video.audio.take(),
                ..video.clone()
            };
            let target = audio_track_for(&next, &sound, link)
                .unwrap_or_else(|| new_audio_track(&mut next, &mut self.track_ids));
            place_clip(&mut next.tracks[target], sound)?;
            detached = true;
        }
        if !detached {
            return Err(CommandError::NothingToEdit);
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

/// The first unlocked audio track where `clip`'s range is free and no clip of `link` lies.
fn audio_track_for(project: &Project, clip: &Clip, link: LinkId) -> Option<usize> {
    project.tracks.iter().position(|t| {
        t.kind == TrackKind::Audio
            && !t.locked
            && is_free(t, clip.start, clip.end(), &[])
            && t.clips.iter().all(|c| c.link != Some(link))
    })
}

/// Creates an empty audio track at the top of the audio section and returns its index.
fn new_audio_track(project: &mut Project, track_ids: &mut IdPool<TrackId>) -> usize {
    let mut track = Track::new(TrackKind::Audio, project.next_track_name(TrackKind::Audio));
    track.id = track_ids.take();
    let index = section_index(project, TrackKind::Audio, Some(0));
    project.tracks.insert(index, track);
    index
}

#[cfg(test)]
mod tests;
