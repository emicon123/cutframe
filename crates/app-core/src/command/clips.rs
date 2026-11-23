//! Commands that put clips on the timeline, or take them off with their media (design
//! `timeline-editing.md` §4.1, §4.9).

use serde::{Deserialize, Serialize};

use super::{Command, CommandError, HistoryLabel};
use crate::media::{MediaId, MediaKind};
use crate::model::{
    CanvasBackground, Clip, ClipId, ClipSource, DEFAULT_COLOR_DURATION, DEFAULT_IMAGE_DURATION,
    Project, Rgba8, Track, TrackId, TrackKind,
};
use crate::props::{AudioProps, VisualProps};
use crate::time::Time;
use crate::timeline::{
    Follow, IdPool, follow_main, normalize_links, place_clip, remove_emptied_tracks, repack_main,
    section_index,
};

/// What a new clip shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NewClipSource {
    /// A media item: the whole source range of a video or audio file, or an image for
    /// [`DEFAULT_IMAGE_DURATION`].
    Media { media: MediaId },
    /// A colour card for [`DEFAULT_COLOR_DURATION`].
    Color { color: Rgba8 },
}

/// The track a drop lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TrackTarget {
    Existing {
        track: TrackId,
    },
    /// A new track created at `index` (top = 0), clamped into the section that fits the first
    /// clip: video tracks above the main track, audio tracks below it.
    New {
        index: u32,
    },
}

/// Where [`AddClips`] puts its clips. Several clips go one after another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Placement {
    /// At the end of the main track; audio-only media at the end of the first unlocked audio
    /// track (a new one at the top of the audio section if there is none).
    Append,
    /// Into the main track at the clip boundary nearest `at`. Later main clips ripple and the
    /// clips on other tracks follow them.
    Insert { at: Time },
    /// On `track` from `start`. On the magnetic main track this is [`Placement::Insert`].
    /// Elsewhere a clip that would overlap fails with [`CommandError::Overlap`].
    Drop { track: TrackTarget, start: Time },
}

/// Which tracks a new clip may go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    /// Video tracks (the main track or an overlay): video, image and colour clips.
    Video,
    /// Audio tracks: audio-only media.
    Audio,
}

impl Lane {
    fn fits(self, kind: TrackKind) -> bool {
        matches!(
            (self, kind),
            (Lane::Video, TrackKind::Video) | (Lane::Audio, TrackKind::Audio)
        )
    }
}

/// Adds media and colour clips in one history entry (`addClips`, design §4.1).
///
/// The clip ids are fixed in [`AddClips::new`], so the caller knows them before `apply` and redo
/// recreates the same ids, as it does for the tracks the command creates. Fails, changing
/// nothing, for unknown media ([`CommandError::MediaNotFound`]), a target track that is locked
/// ([`CommandError::TrackLocked`]) or of the wrong kind ([`CommandError::TrackKindMismatch`]),
/// an occupied range ([`CommandError::Overlap`]) and an empty source list.
#[derive(Debug)]
pub struct AddClips {
    sources: Vec<NewClipSource>,
    placement: Placement,
    ids: Vec<ClipId>,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl AddClips {
    pub fn new(sources: Vec<NewClipSource>, placement: Placement) -> Self {
        Self {
            ids: sources.iter().map(|_| ClipId::new()).collect(),
            sources,
            placement,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    /// The ids of the new clips, one per source, in source order.
    pub fn clip_ids(&self) -> &[ClipId] {
        &self.ids
    }

    /// The new clips (at start 0) and the tracks they may go on.
    fn clips(&self, project: &Project) -> Result<Vec<(Clip, Lane)>, CommandError> {
        self.sources
            .iter()
            .zip(&self.ids)
            .map(|(source, &id)| {
                let (mut clip, lane) = new_clip(project, source)?;
                clip.id = id;
                Ok((clip, lane))
            })
            .collect()
    }
}

/// A clip for `source` at start 0, with the props its kind of track needs.
fn new_clip(project: &Project, source: &NewClipSource) -> Result<(Clip, Lane), CommandError> {
    let id = match *source {
        NewClipSource::Color { color } => {
            let clip = Clip::color(color.to_string(), Time::ZERO, DEFAULT_COLOR_DURATION, color);
            return Ok((clip, Lane::Video));
        }
        NewClipSource::Media { media } => media,
    };
    let asset = project
        .media_asset(id)
        .ok_or(CommandError::MediaNotFound(id))?;
    let name = asset.file_name();
    let name = name
        .rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_owned();
    let has_audio = asset.kind != MediaKind::Image && !asset.info.audio.is_empty();
    let (duration, lane) = match asset.kind {
        MediaKind::Image => (DEFAULT_IMAGE_DURATION, Lane::Video),
        MediaKind::Video => (asset.info.duration.unwrap_or_default(), Lane::Video),
        MediaKind::Audio => (asset.info.duration.unwrap_or_default(), Lane::Audio),
    };
    let clip = Clip {
        id: ClipId::new(),
        name,
        start: Time::ZERO,
        duration,
        source: ClipSource::Media {
            media: id,
            source_in: Time::ZERO,
        },
        link: None,
        visual: (lane == Lane::Video).then(VisualProps::default),
        audio: has_audio.then(AudioProps::default),
    };
    Ok((clip, lane))
}

impl Command for AddClips {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::AddClips
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.sources.is_empty() {
            return Err(CommandError::NothingToEdit);
        }
        self.track_ids.rewind();
        let clips = self.clips(project)?;
        let mut next = project.clone();
        let rippled = place(&mut next, clips, self.placement, &mut self.track_ids)?;
        if rippled {
            repack_main(&mut next);
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
        remove_emptied_tracks(project, &mut next);
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

/// Puts `clips` where `placement` says. Returns whether main-track clips may have shifted, so
/// the caller repacks and lets the other tracks follow.
fn place(
    project: &mut Project,
    clips: Vec<(Clip, Lane)>,
    placement: Placement,
    track_ids: &mut IdPool<TrackId>,
) -> Result<bool, CommandError> {
    let main = project.main_index().ok_or(CommandError::MainTrack)?;
    match placement {
        Placement::Append => {
            for (clip, lane) in clips {
                let index = match lane {
                    Lane::Video => unlocked(project, main)?,
                    Lane::Audio => match project
                        .tracks
                        .iter()
                        .position(|t| t.kind == TrackKind::Audio && !t.locked)
                    {
                        Some(index) => index,
                        None => new_track(project, TrackKind::Audio, Some(main + 1), track_ids),
                    },
                };
                append(&mut project.tracks[index], clip);
            }
            Ok(false)
        }
        Placement::Insert { at } => {
            insert_main(project, clips, at)?;
            Ok(true)
        }
        Placement::Drop { track, start } => {
            let index = match track {
                TrackTarget::Existing { track } => project
                    .track_index(track)
                    .ok_or(CommandError::TrackNotFound(track))?,
                TrackTarget::New { index } => {
                    let kind = match clips.first() {
                        Some((_, Lane::Audio)) => TrackKind::Audio,
                        _ => TrackKind::Video,
                    };
                    new_track(project, kind, Some(index as usize), track_ids)
                }
            };
            if index == project.main_index().ok_or(CommandError::MainTrack)? && project.magnetic {
                insert_main(project, clips, start)?;
                return Ok(true);
            }
            let index = unlocked(project, index)?;
            let track = &mut project.tracks[index];
            let mut at = start;
            for (mut clip, lane) in clips {
                if !lane.fits(track.kind) {
                    return Err(CommandError::TrackKindMismatch);
                }
                clip.start = at;
                at = clip.end();
                place_clip(track, clip)?;
            }
            Ok(false)
        }
    }
}

/// `index` if that track is unlocked, else [`CommandError::TrackLocked`].
fn unlocked(project: &Project, index: usize) -> Result<usize, CommandError> {
    let track = &project.tracks[index];
    if track.locked {
        return Err(CommandError::TrackLocked(track.id));
    }
    Ok(index)
}

/// Creates an empty track of `kind` in its section and returns its index.
fn new_track(
    project: &mut Project,
    kind: TrackKind,
    index: Option<usize>,
    track_ids: &mut IdPool<TrackId>,
) -> usize {
    let mut track = Track::new(kind, project.next_track_name(kind));
    track.id = track_ids.take();
    let index = section_index(project, kind, index);
    project.tracks.insert(index, track);
    index
}

/// Puts `clip` after the last clip of `track`.
fn append(track: &mut Track, mut clip: Clip) {
    clip.start = track.clips.last().map_or(Time::ZERO, Clip::end);
    track.clips.push(clip);
}

/// Inserts `clips` into the main track at the clip boundary nearest `at` (ties: the earlier
/// one), shifting the later main clips by their total length.
fn insert_main(
    project: &mut Project,
    clips: Vec<(Clip, Lane)>,
    at: Time,
) -> Result<(), CommandError> {
    if clips.iter().any(|(_, lane)| *lane != Lane::Video) {
        return Err(CommandError::TrackKindMismatch);
    }
    let main = unlocked(
        project,
        project.main_index().ok_or(CommandError::MainTrack)?,
    )?;
    let track = &mut project.tracks[main];
    let boundary = track
        .clips
        .iter()
        .flat_map(|c| [c.start, c.end()])
        .chain([Time::ZERO])
        .min_by_key(|b| ((*b - at).flicks().unsigned_abs(), *b))
        .unwrap_or(Time::ZERO);
    let total = clips
        .iter()
        .fold(Time::ZERO, |sum, (clip, _)| sum + clip.duration);
    for clip in track.clips.iter_mut().filter(|c| c.start >= boundary) {
        clip.start = clip.start + total;
    }
    let index = track.clips.partition_point(|c| c.start < boundary);
    let mut start = boundary;
    let new = clips.into_iter().map(|(mut clip, _)| {
        clip.start = start;
        start = clip.end();
        clip
    });
    track.clips.splice(index..index, new);
    Ok(())
}

/// Removes every clip that uses `media` and clears a canvas background that shows it (back to
/// black), in the media design's remove-media `Batch` (design §4.9). The main track repacks, the
/// other tracks follow to the removal point (never deleted), links are normalised and emptied
/// tracks removed. Fails with [`CommandError::TrackLocked`] when a clip of the media is on a locked
/// track. Succeeds without a change when nothing uses the media.
#[derive(Debug)]
pub struct RemoveClipsOfMedia {
    media: MediaId,
    track_ids: IdPool<TrackId>,
    previous: Option<(Vec<Track>, CanvasBackground)>,
}

impl RemoveClipsOfMedia {
    pub fn new(media: MediaId) -> Self {
        Self {
            media,
            track_ids: IdPool::default(),
            previous: None,
        }
    }
}

impl Command for RemoveClipsOfMedia {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::RemoveMedia
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let uses = |c: &Clip| c.source.media() == Some(self.media);
        if let Some(track) = project
            .tracks
            .iter()
            .find(|t| t.locked && t.clips.iter().any(uses))
        {
            return Err(CommandError::TrackLocked(track.id));
        }
        self.track_ids.rewind();
        let mut next = project.clone();
        for track in &mut next.tracks {
            track.clips.retain(|c| !uses(c));
        }
        if next.background == (CanvasBackground::Image { media: self.media }) {
            next.background = CanvasBackground::default();
        }
        repack_main(&mut next);
        follow_main(
            project,
            &mut next,
            Follow {
                moved: &[],
                delete_inside: false,
                track_ids: &mut self.track_ids,
            },
        )?;
        normalize_links(&mut next);
        remove_emptied_tracks(project, &mut next);
        self.previous = Some((
            std::mem::replace(&mut project.tracks, next.tracks),
            std::mem::replace(&mut project.background, next.background),
        ));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some((tracks, background)) = self.previous.take() {
            project.tracks = tracks;
            project.background = background;
        }
    }
}

#[cfg(test)]
mod tests;
