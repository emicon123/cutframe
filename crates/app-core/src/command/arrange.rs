//! Moving and trimming clips: the timeline's drag gestures (design `timeline-editing.md` §4.2,
//! §4.3, ADR-0006).
//!
//! Both commands take absolute values (the anchor's new start, the edge's new time), so every
//! gesture update is a fresh command applied to the gesture's base. Both work on a clone of the
//! project and swap the tracks in only when the result is valid, so a failed apply changes nothing.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::cut::set_source_in;
use super::{Command, CommandError, HistoryLabel, TrackTarget};
use crate::media::MediaKind;
use crate::model::{Clip, ClipId, ClipSource, LinkId, Project, Track, TrackId};
use crate::time::{MAX_IPC_FLICKS, Time};
use crate::timeline::{
    Follow, IdPool, expand_links, follow_main, main_changed, place_clip, remove_emptied_tracks,
    repack_main, retime_keyframes, section_index,
};

/// Which edge of a clip a trim moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Start,
    End,
}

/// Moves clips in time and, when they all sit on one track, to another track (`moveClips`,
/// design §4.2).
///
/// The anchor clip goes to `start`; every other clip keeps its time offset from the anchor. Clips
/// linked to a moved clip move with it in time and stay on their own tracks (§3.5).
///
/// - **Vertical move:** allowed only when every listed clip is on the anchor's track or linked to
///   one that is; otherwise `track` is ignored and the move is time-only. The clips of the
///   anchor's track go to `track` (an existing track of the same kind, or a new one created at
///   `New.index` in the section of that kind).
/// - **Magnetic main track:** a clip moved onto it is inserted at the index given by `start` (the
///   number of the other main clips whose midpoint lies before it) and the track repacks. Clips
///   linked to a moved main clip shift by that clip's actual change.
/// - **Elsewhere:** a clip that would start before 0 or overlap another clip fails with
///   [`CommandError::Overlap`].
/// - When main-track clips shift, the clips on other tracks follow (§3.7).
/// - With `remove_emptied_tracks` (the IPC handler sets it on commit only), tracks the move left
///   empty are removed.
///
/// Fails, changing nothing, with [`CommandError::ClipNotFound`], [`CommandError::TrackNotFound`],
/// [`CommandError::TrackLocked`] (a moved clip or the target on a locked track),
/// [`CommandError::TrackKindMismatch`] (target of another kind), [`CommandError::Overlap`] or
/// [`CommandError::InvalidResult`] (e.g. two members of a link group on one track).
#[derive(Debug)]
pub struct MoveClips {
    clips: Vec<ClipId>,
    anchor: ClipId,
    start: Time,
    track: TrackTarget,
    remove_emptied_tracks: bool,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl MoveClips {
    /// The anchor counts as listed even when `clips` leaves it out.
    pub fn new(
        clips: Vec<ClipId>,
        anchor: ClipId,
        start: Time,
        track: TrackTarget,
        remove_emptied_tracks: bool,
    ) -> Self {
        Self {
            clips,
            anchor,
            start,
            track,
            remove_emptied_tracks,
            track_ids: IdPool::default(),
            previous: None,
        }
    }
}

impl Command for MoveClips {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::MoveClips
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.moved(project)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

impl MoveClips {
    /// The project after the move.
    fn moved(&mut self, project: &Project) -> Result<Project, CommandError> {
        let (anchor_index, anchor) = project
            .clip(self.anchor)
            .ok_or(CommandError::ClipNotFound(self.anchor))?;
        let anchor_track = project.tracks[anchor_index].id;
        let anchor_start = anchor.start;
        let delta = self.start - anchor_start;

        let mut listed = self.clips.clone();
        if !listed.contains(&self.anchor) {
            listed.push(self.anchor);
        }
        let moving = expand_links(project, &listed)?;
        let track_of: HashMap<ClipId, usize> = project
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(i, t)| t.clips.iter().map(move |c| (c.id, i)))
            .collect();
        for id in &moving {
            let track = &project.tracks[track_of[id]];
            if track.locked {
                return Err(CommandError::TrackLocked(track.id));
            }
        }

        // A vertical move needs every listed clip on the anchor's track or linked to one there.
        let anchor_links: HashSet<LinkId> = project.tracks[anchor_index]
            .clips
            .iter()
            .filter(|c| listed.contains(&c.id))
            .filter_map(|c| c.link)
            .collect();
        let vertical = listed.iter().all(|id| {
            track_of[id] == anchor_index
                || project
                    .clip(*id)
                    .and_then(|(_, c)| c.link)
                    .is_some_and(|l| anchor_links.contains(&l))
        });

        let mut next = project.clone();
        let target = if vertical {
            self.target(&mut next, anchor_track)?
        } else {
            anchor_track
        };

        // Take the moving clips out, remembering where they came from.
        let moving_set: HashSet<ClipId> = moving.iter().copied().collect();
        let mut taken: Vec<(TrackId, Clip)> = Vec::with_capacity(moving.len());
        for track in &mut next.tracks {
            let (out, kept): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut track.clips)
                .into_iter()
                .partition(|c| moving_set.contains(&c.id));
            track.clips = kept;
            taken.extend(out.into_iter().map(|c| (track.id, c)));
        }
        let dest = |origin: TrackId| {
            if origin == anchor_track {
                target
            } else {
                origin
            }
        };

        // Clips going onto the magnetic main track are inserted as a block, then repacked.
        let main = next.main_track;
        let magnetic = next.magnetic;
        let (to_main, others): (Vec<_>, Vec<_>) = taken
            .into_iter()
            .partition(|(origin, _)| magnetic && dest(*origin) == main);
        let mut main_deltas: HashMap<LinkId, Time> = HashMap::new();
        let mut effective = delta;
        if !to_main.is_empty() {
            let block_start = to_main
                .iter()
                .find(|(_, c)| c.id == self.anchor)
                .map_or_else(|| to_main[0].1.start + delta, |_| self.start);
            let main_index = next.main_index().ok_or(CommandError::MainTrack)?;
            let track = &mut next.tracks[main_index];
            let index = track
                .clips
                .iter()
                .filter(|c| c.start + Time(c.duration.flicks() / 2) < block_start)
                .count();
            let old: Vec<(ClipId, Time)> = to_main.iter().map(|(_, c)| (c.id, c.start)).collect();
            track
                .clips
                .splice(index..index, to_main.into_iter().map(|(_, c)| c));
            repack_main(&mut next);
            let main_clips = &next.main().ok_or(CommandError::MainTrack)?.clips;
            for (id, old_start) in old {
                let clip = main_clips.iter().find(|c| c.id == id).expect("inserted");
                let actual = clip.start - old_start;
                if let Some(link) = clip.link {
                    main_deltas.insert(link, actual);
                }
                if id == self.anchor {
                    effective = actual;
                }
            }
        }

        let mut placed: Vec<(TrackId, Clip)> = others
            .into_iter()
            .map(|(origin, mut clip)| {
                let shift = clip
                    .link
                    .and_then(|l| main_deltas.get(&l).copied())
                    .unwrap_or(effective);
                clip.start = clip.start + shift;
                (dest(origin), clip)
            })
            .collect();
        placed.sort_by_key(|(_, c)| (c.start, c.id));
        for (track_id, clip) in placed {
            if clip.start < Time::ZERO {
                return Err(CommandError::Overlap);
            }
            let track = next
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
                .expect("destination exists");
            place_clip(track, clip)?;
        }
        repack_main(&mut next);

        if main_changed(project, &next) {
            follow_main(
                project,
                &mut next,
                Follow {
                    moved: &moving,
                    delete_inside: false,
                    track_ids: &mut self.track_ids,
                },
            )?;
        }
        if self.remove_emptied_tracks {
            remove_emptied_tracks(project, &mut next);
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }

    /// The track the anchor's clips go to, created in `next` for [`TrackTarget::New`].
    fn target(
        &mut self,
        next: &mut Project,
        anchor_track: TrackId,
    ) -> Result<TrackId, CommandError> {
        let kind = next.track(anchor_track).expect("anchor track exists").kind;
        match self.track {
            TrackTarget::Existing { track } => {
                let target = next
                    .track(track)
                    .ok_or(CommandError::TrackNotFound(track))?;
                if target.locked {
                    return Err(CommandError::TrackLocked(track));
                }
                if target.kind != kind {
                    return Err(CommandError::TrackKindMismatch);
                }
                Ok(track)
            }
            TrackTarget::New { index } => {
                let mut track = Track::new(kind, next.next_track_name(kind));
                track.id = self.track_ids.take();
                let id = track.id;
                let at = section_index(next, kind, Some(index as usize));
                next.tracks.insert(at, track);
                Ok(id)
            }
        }
    }
}

/// Moves one edge of a clip to `time` (`trimClip`, design §4.3).
///
/// The edge is **clamped**, never rejected: to the source range of video and audio media
/// (`source_in` at least 0, source end at most the media's duration, with the clip's speed and
/// direction: [`Clip::extend_room`]; images, freeze, colour and text clips have no limit), to at
/// least one project frame (or the clip's current length if it is shorter), to the neighbouring
/// clips (except on the magnetic main track), and to the same limits of every linked clip whose
/// same edge was at the same time: those members trim by the same amount (§3.5).
///
/// A trim keeps the content in place: the source range follows the edge ([`Clip::sub_source_in`];
/// forward a start trim moves `source_in`, reversed an end trim does). On the magnetic main
/// track the clip keeps its start instead (its content shifts) and the later main clips
/// ripple; its trimmed linked members do the same, so they stay in sync. When a main clip changes,
/// the clips on other tracks follow (§3.7). Keyframes are retimed and fades shortened to fit.
///
/// Fails, changing nothing, with [`CommandError::ClipNotFound`], [`CommandError::TrackLocked`] (the
/// clip or a trimmed member is on a locked track) and [`CommandError::InvalidResult`].
#[derive(Debug)]
pub struct TrimClip {
    clip: ClipId,
    edge: Edge,
    time: Time,
    remove_emptied_tracks: bool,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl TrimClip {
    pub fn new(clip: ClipId, edge: Edge, time: Time, remove_emptied_tracks: bool) -> Self {
        Self {
            clip,
            edge,
            time,
            remove_emptied_tracks,
            track_ids: IdPool::default(),
            previous: None,
        }
    }
}

impl Command for TrimClip {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::TrimClip
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.trimmed(project)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

/// A trimmed clip and what limits it.
struct Member {
    track: usize,
    index: usize,
    /// On the magnetic main track: no neighbour limits, the later clips ripple.
    rippling_track: bool,
    /// Length of the media's source for video and audio; `None` for images, colour and text.
    source_length: Option<Time>,
    /// Whether a start trim moves `source_in` (video and audio media).
    has_source_in: bool,
}

impl TrimClip {
    fn edge_of(&self, clip: &Clip) -> Time {
        match self.edge {
            Edge::Start => clip.start,
            Edge::End => clip.end(),
        }
    }

    /// The project after the trim.
    fn trimmed(&mut self, project: &Project) -> Result<Project, CommandError> {
        let (_, clip) = project
            .clip(self.clip)
            .ok_or(CommandError::ClipNotFound(self.clip))?;
        let edge = self.edge_of(clip);
        let group = expand_links(project, &[self.clip])?;
        let main_index = project.main_index().ok_or(CommandError::MainTrack)?;

        let mut members = Vec::new();
        for (track_index, track) in project.tracks.iter().enumerate() {
            for (index, c) in track.clips.iter().enumerate() {
                if !group.contains(&c.id) || self.edge_of(c) != edge {
                    continue;
                }
                if track.locked {
                    return Err(CommandError::TrackLocked(track.id));
                }
                let media = match &c.source {
                    ClipSource::Media { media, .. } => project.media_asset(*media),
                    _ => None,
                };
                let timed = media.filter(|m| m.kind != MediaKind::Image);
                members.push(Member {
                    track: track_index,
                    index,
                    rippling_track: project.magnetic && track_index == main_index,
                    source_length: timed.and_then(|m| m.info.duration),
                    has_source_in: timed.is_some(),
                });
            }
        }
        // A start trim that ripples keeps the start of every member, so they stay in sync.
        let ripple = members.iter().any(|m| m.rippling_track);
        let frame = project.settings.frame_rate.frame_duration();

        let (mut lo, mut hi) = (i64::MIN, i64::MAX);
        for m in &members {
            let clips = &project.tracks[m.track].clips;
            let c = &clips[m.index];
            let min_len = frame.min(c.duration).flicks();
            let prev = m.index.checked_sub(1).map(|i| &clips[i]);
            let next = clips.get(m.index + 1);
            let neighbours = !m.rippling_track;
            let (start_room, end_room) = m.source_length.map_or((i64::MAX, i64::MAX), |len| {
                let (start, end) = c.extend_room(len);
                (start.flicks(), end.flicks())
            });
            match self.edge {
                // `d` > 0 shortens the clip from the start.
                Edge::Start => {
                    hi = hi.min(c.duration.flicks() - min_len);
                    if m.has_source_in {
                        lo = lo.max(-start_room);
                    }
                    if ripple {
                        // The start stays and the end moves by -d.
                        if let Some(next) = next.filter(|_| neighbours) {
                            lo = lo.max((c.end() - next.start).flicks());
                        }
                    } else {
                        lo = lo.max(-c.start.flicks());
                        if let Some(prev) = prev.filter(|_| neighbours) {
                            lo = lo.max((prev.end() - c.start).flicks());
                        }
                    }
                }
                // `d` > 0 lengthens the clip at the end.
                Edge::End => {
                    lo = lo.max(min_len - c.duration.flicks());
                    hi = hi.min(end_room);
                    if let Some(next) = next.filter(|_| neighbours) {
                        hi = hi.min((next.start - c.end()).flicks());
                    }
                    hi = hi.min(MAX_IPC_FLICKS - c.end().flicks());
                }
            }
        }
        // `0` is within the limits of a valid project, so `lo <= hi` there; stay put otherwise.
        let requested = (self.time - edge).flicks();
        let d = Time(if lo <= hi { requested.clamp(lo, hi) } else { 0 });
        if d == Time::ZERO {
            return Ok(project.clone());
        }

        let mut next = project.clone();
        for m in &members {
            let c = &mut next.tracks[m.track].clips[m.index];
            match self.edge {
                Edge::Start => {
                    if m.has_source_in {
                        let source_in = c.sub_source_in(d, c.duration - d);
                        set_source_in(c, source_in);
                    }
                    c.duration = c.duration - d;
                    if !ripple {
                        c.start = c.start + d;
                    }
                    retime_keyframes(c, d);
                }
                Edge::End => {
                    if m.has_source_in {
                        // Reversed clips keep their range's top: the end edge moves `source_in`.
                        let source_in = c.sub_source_in(Time::ZERO, c.duration + d);
                        set_source_in(c, source_in);
                    }
                    c.duration = c.duration + d;
                    retime_keyframes(c, Time::ZERO);
                }
            }
        }

        if members.iter().any(|m| m.track == main_index) {
            repack_main(&mut next);
            // The whole link group is placed by the trim; its untrimmed members stay put.
            follow_main(
                project,
                &mut next,
                Follow {
                    moved: &group,
                    delete_inside: false,
                    track_ids: &mut self.track_ids,
                },
            )?;
        }
        if self.remove_emptied_tracks {
            remove_emptied_tracks(project, &mut next);
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
