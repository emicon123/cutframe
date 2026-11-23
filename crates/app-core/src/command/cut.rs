//! Cutting the timeline: split, delete, ripple delete, delete gap, trim to playhead and the
//! magnetic toggle (design `timeline-editing.md` §4.4–4.7, §4.17).
//!
//! Like the drag commands, each works on a copy of the project and swaps the tracks in only when
//! the result is valid, so a failed apply changes nothing.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Command, CommandError, Edge, HistoryLabel, TrimClip};
use crate::media::MediaKind;
use crate::model::{Clip, ClipId, ClipSource, LinkId, Project, Track, TrackId};
use crate::time::Time;
use crate::timeline::{
    Follow, IdPool, expand_links, follow_main, main_changed, normalize_links,
    remove_emptied_tracks, repack_main, retime_keyframes, shift_clips,
};

/// Which part of a clip [`TrimToPlayhead`] deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// Delete left of the playhead (Q): the start edge moves to the playhead.
    Left,
    /// Delete right of the playhead (W): the end edge moves to the playhead.
    Right,
}

/// The track of every clip, by id.
fn track_of(project: &Project) -> HashMap<ClipId, usize> {
    project
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(i, t)| t.clips.iter().map(move |c| (c.id, i)))
        .collect()
}

/// [`CommandError::TrackLocked`] for the first clip of `ids` that is on a locked track.
fn ensure_unlocked<'a>(
    project: &Project,
    ids: impl IntoIterator<Item = &'a ClipId>,
) -> Result<(), CommandError> {
    let track_of = track_of(project);
    for id in ids {
        let track = &project.tracks[track_of[id]];
        if track.locked {
            return Err(CommandError::TrackLocked(track.id));
        }
    }
    Ok(())
}

/// Whether a start trim or split moves the clip's `source_in`: video and audio media. Images
/// keep `source_in == 0`; colour and text clips have none.
fn has_source_in(project: &Project, clip: &Clip) -> bool {
    match &clip.source {
        ClipSource::Media { media, .. } => project
            .media_asset(*media)
            .is_some_and(|m| m.kind != MediaKind::Image),
        _ => false,
    }
}

/// Splits clips at a time (`splitClips`, design §4.4).
///
/// With `clips` empty, every clip under `at` on an unlocked track splits (a link group with a
/// member on a locked track that would split is skipped). Otherwise the listed clips and their
/// linked members split, and a member on a locked track fails the edit with
/// [`CommandError::TrackLocked`].
///
/// A clip splits only when `at` lies inside it with both halves at least one project frame long;
/// others are skipped. The right half gets a new id, `source_in` advanced by the left half's
/// length (video and audio), the same props with keyframes partitioned, and no fade-in; the left
/// half loses its fade-out. The right halves of one link group share one new link. Nothing
/// moves, so nothing follows. Fails with [`CommandError::NothingToSplit`] when no clip split.
#[derive(Debug)]
pub struct SplitClips {
    clips: Vec<ClipId>,
    at: Time,
    ids: IdPool<ClipId>,
    links: IdPool<LinkId>,
    previous: Option<Vec<Track>>,
}

impl SplitClips {
    pub fn new(clips: Vec<ClipId>, at: Time) -> Self {
        Self {
            clips,
            at,
            ids: IdPool::default(),
            links: IdPool::default(),
            previous: None,
        }
    }

    /// The clips to split.
    fn targets(&self, project: &Project) -> Result<HashSet<ClipId>, CommandError> {
        let frame = project.settings.frame_rate.frame_duration();
        let at = self.at;
        let splits = |c: &Clip| c.start + frame <= at && at + frame <= c.end();
        let track_of = track_of(project);
        let locked = |id: &ClipId| project.tracks[track_of[id]].locked;
        let splitting = |ids: Vec<ClipId>| -> Vec<ClipId> {
            ids.into_iter()
                .filter(|id| project.clip(*id).is_some_and(|(_, c)| splits(c)))
                .collect()
        };
        if self.clips.is_empty() {
            let mut targets = HashSet::new();
            for track in project.tracks.iter().filter(|t| !t.locked) {
                for clip in track.clips.iter().filter(|c| splits(c)) {
                    let group = splitting(expand_links(project, &[clip.id])?);
                    if !group.iter().any(locked) {
                        targets.extend(group);
                    }
                }
            }
            Ok(targets)
        } else {
            let group = splitting(expand_links(project, &self.clips)?);
            ensure_unlocked(project, &group)?;
            Ok(group.into_iter().collect())
        }
    }
}

impl Command for SplitClips {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SplitClips
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.ids.rewind();
        self.links.rewind();
        let targets = self.targets(project)?;
        if targets.is_empty() {
            return Err(CommandError::NothingToSplit);
        }
        let mut next = project.clone();
        let mut new_links: HashMap<LinkId, LinkId> = HashMap::new();
        for track in &mut next.tracks {
            let mut split = Vec::with_capacity(track.clips.len());
            for mut clip in std::mem::take(&mut track.clips) {
                if !targets.contains(&clip.id) {
                    split.push(clip);
                    continue;
                }
                let shifts_source = has_source_in(project, &clip);
                let mut right = split_clip(&mut clip, self.at, self.ids.take(), shifts_source);
                right.link = clip
                    .link
                    .map(|l| *new_links.entry(l).or_insert_with(|| self.links.take()));
                split.push(clip);
                split.push(right);
            }
            track.clips = split;
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

/// Cuts `left` at `at` (inside it) and returns the right half with id `id`.
fn split_clip(left: &mut Clip, at: Time, id: ClipId, shifts_source: bool) -> Clip {
    let offset = at - left.start;
    let mut right = left.clone();
    right.id = id;
    right.start = at;
    right.duration = left.end() - at;
    if shifts_source && let ClipSource::Media { source_in, .. } = &mut right.source {
        *source_in = *source_in + offset;
    }
    left.duration = offset;
    if let Some(audio) = &mut left.audio {
        audio.fade_out = Time::ZERO;
    }
    if let Some(audio) = &mut right.audio {
        audio.fade_in = Time::ZERO;
    }
    retime_keyframes(left, Time::ZERO);
    retime_keyframes(&mut right, offset);
    right
}

/// Removes clips (`deleteClips`, `rippleDeleteClips`, `cutClips`; design §4.5, §3.7).
///
/// The listed clips and their linked members are removed; one on a locked track fails the edit
/// with [`CommandError::TrackLocked`].
///
/// - **Main-track clips removed:** on the magnetic main track the track repacks, so Delete always
///   closes the gap there. With `ripple` on a non-magnetic main track the later main clips shift
///   left by the removed length before them. When the main track closes up (either case), the
///   clips on other tracks follow, and those lying entirely inside a removed main clip are
///   deleted with it (§3.7). A plain Delete on a non-magnetic main track leaves the gap and moves
///   nothing else.
/// - **Only other tracks:** with `ripple`, each track that lost clips closes its gaps: its later
///   clips shift left by the removed length before them, and their linked members shift with
///   them (a group with a member on the main track stays, as the main track holds it). A shift
///   into an occupied spot fails with [`CommandError::Overlap`].
///
/// Links left with one clip are cleared and tracks the delete emptied are removed.
#[derive(Debug)]
pub struct DeleteClips {
    clips: Vec<ClipId>,
    ripple: bool,
    label: HistoryLabel,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl DeleteClips {
    /// Delete (`ripple == false`) or ripple delete.
    pub fn new(clips: Vec<ClipId>, ripple: bool) -> Self {
        let label = if ripple {
            HistoryLabel::RippleDeleteClips
        } else {
            HistoryLabel::DeleteClips
        };
        Self {
            clips,
            ripple,
            label,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    /// The delete half of a cut: a plain delete (rippling on the magnetic main track), labelled
    /// `cutClips`.
    pub fn cut(clips: Vec<ClipId>) -> Self {
        Self {
            label: HistoryLabel::CutClips,
            ..Self::new(clips, false)
        }
    }

    fn deleted(&mut self, project: &Project) -> Result<Project, CommandError> {
        if self.clips.is_empty() {
            return Err(CommandError::NothingToEdit);
        }
        let ids: HashSet<ClipId> = expand_links(project, &self.clips)?.into_iter().collect();
        ensure_unlocked(project, &ids)?;

        let mut next = project.clone();
        let main = next.main_track;
        // Removed spans per track.
        let mut removed: HashMap<TrackId, Vec<(Time, Time)>> = HashMap::new();
        for track in &mut next.tracks {
            track.clips.retain(|c| {
                let keep = !ids.contains(&c.id);
                if !keep {
                    removed
                        .entry(track.id)
                        .or_default()
                        .push((c.start, c.end()));
                }
                keep
            });
        }

        if let Some(spans) = removed.get(&main) {
            let closes = next.magnetic || self.ripple;
            if next.magnetic {
                repack_main(&mut next);
            } else if self.ripple {
                let track = next.main().expect("main track exists");
                let deltas = ripple_deltas(track, spans);
                shift_clips(&mut next, &deltas)?;
            }
            if closes {
                follow_main(
                    project,
                    &mut next,
                    Follow {
                        moved: &[],
                        delete_inside: true,
                        track_ids: &mut self.track_ids,
                    },
                )?;
            }
        } else if self.ripple {
            ripple_other_tracks(&mut next, &removed)?;
        }
        normalize_links(&mut next);
        remove_emptied_tracks(project, &mut next);
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

impl Command for DeleteClips {
    fn label(&self) -> HistoryLabel {
        self.label
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.deleted(project)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

/// For the clips left on `track` after removing `spans`: how far each later clip moves left to
/// close them (the removed length before it). Clips that do not move are left out.
fn ripple_deltas(track: &Track, spans: &[(Time, Time)]) -> HashMap<ClipId, Time> {
    track
        .clips
        .iter()
        .filter_map(|c| {
            let before: i64 = spans
                .iter()
                .filter(|(_, end)| *end <= c.start)
                .map(|(start, end)| (*end - *start).flicks())
                .sum();
            (before > 0).then_some((c.id, Time(-before)))
        })
        .collect()
}

/// Ripple on the non-main tracks in `removed` (the main track lost nothing): each closes its
/// gaps, and linked members on other tracks shift with the clips they are linked to (by the
/// smallest shift in the group, so the group stays in sync).
fn ripple_other_tracks(
    next: &mut Project,
    removed: &HashMap<TrackId, Vec<(Time, Time)>>,
) -> Result<(), CommandError> {
    let mut deltas: HashMap<ClipId, Time> = HashMap::new();
    for track in &next.tracks {
        if let Some(spans) = removed.get(&track.id) {
            deltas.extend(ripple_deltas(track, spans));
        }
    }
    // A link group moves as one: by its smallest shift, unless the main track holds it.
    let mut groups: HashMap<LinkId, Time> = HashMap::new();
    let mut held: HashSet<LinkId> = HashSet::new();
    for track in &next.tracks {
        for clip in &track.clips {
            let Some(link) = clip.link else { continue };
            if track.id == next.main_track {
                held.insert(link);
            }
            if let Some(&d) = deltas.get(&clip.id) {
                groups
                    .entry(link)
                    .and_modify(|g| *g = (*g).max(d))
                    .or_insert(d);
            }
        }
    }
    for track in &next.tracks {
        for clip in &track.clips {
            let Some(link) = clip.link else { continue };
            if held.contains(&link) {
                deltas.remove(&clip.id);
            } else if let Some(&d) = groups.get(&link) {
                if track.locked {
                    return Err(CommandError::TrackLocked(track.id));
                }
                deltas.insert(clip.id, d);
            }
        }
    }
    shift_clips(next, &deltas)
}

/// Closes the gap at `at` on `track` (`deleteGap`, design §4.6).
///
/// The gap is the free span between two clips, or before the first clip, that contains `at`.
/// The clips after it on that track, and their linked members, move left by its length. When
/// main-track clips move, the clips on other tracks follow (§3.7). Fails with
/// [`CommandError::NoGap`] when `at` is not in a gap, [`CommandError::TrackLocked`] for a locked
/// track or linked member, and [`CommandError::Overlap`] when a linked member would collide.
#[derive(Debug)]
pub struct DeleteGap {
    track: TrackId,
    at: Time,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl DeleteGap {
    pub fn new(track: TrackId, at: Time) -> Self {
        Self {
            track,
            at,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    fn closed(&mut self, project: &Project) -> Result<Project, CommandError> {
        let track = project
            .track(self.track)
            .ok_or(CommandError::TrackNotFound(self.track))?;
        if track.locked {
            return Err(CommandError::TrackLocked(track.id));
        }
        let mut gap_start = Time::ZERO;
        let mut gap = None;
        for clip in &track.clips {
            if gap_start <= self.at && self.at < clip.start {
                gap = Some((gap_start, clip.start));
                break;
            }
            gap_start = clip.end();
        }
        let (start, end) = gap.ok_or(CommandError::NoGap)?;
        let following: Vec<ClipId> = track
            .clips
            .iter()
            .filter(|c| c.start >= end)
            .map(|c| c.id)
            .collect();
        let moving = expand_links(project, &following)?;
        ensure_unlocked(project, &moving)?;
        let deltas: HashMap<ClipId, Time> = moving.iter().map(|&id| (id, start - end)).collect();
        let mut next = project.clone();
        shift_clips(&mut next, &deltas)?;
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
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

impl Command for DeleteGap {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::DeleteGap
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.closed(project)?;
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

/// Deletes the part of clips left or right of `at` (`trimToPlayhead`, Q / W, design §4.7).
///
/// `Left` moves the start edge to `at`, `Right` the end edge, with the clamping, linked members,
/// ripple and following of [`TrimClip`]: on the magnetic main track a left trim keeps the clip's
/// start, so the content after `at` moves to where the clip started. With `clips` empty, the
/// main-track clip under `at` is trimmed. Clips that `at` does not lie strictly inside are
/// skipped; clips on other tracks trim before main-track clips, so they are not moved away from
/// `at` first. Fails with [`CommandError::NothingToEdit`] when nothing changed.
#[derive(Debug)]
pub struct TrimToPlayhead {
    clips: Vec<ClipId>,
    at: Time,
    side: Side,
    /// One trim per clip, kept so a redo hands out the same ids as the first apply.
    trims: HashMap<ClipId, TrimClip>,
    previous: Option<Vec<Track>>,
}

impl TrimToPlayhead {
    pub fn new(clips: Vec<ClipId>, at: Time, side: Side) -> Self {
        Self {
            clips,
            at,
            side,
            trims: HashMap::new(),
            previous: None,
        }
    }

    fn crosses(&self, clip: &Clip) -> bool {
        clip.start < self.at && self.at < clip.end()
    }

    /// The clips to trim, the ones off the main track first.
    fn targets(&self, project: &Project) -> Result<Vec<ClipId>, CommandError> {
        if self.clips.is_empty() {
            let main = project.main().ok_or(CommandError::MainTrack)?;
            return Ok(main
                .clips
                .iter()
                .filter(|c| self.crosses(c))
                .map(|c| c.id)
                .collect());
        }
        let mut targets = Vec::with_capacity(self.clips.len());
        for &id in &self.clips {
            let (track, _) = project.clip(id).ok_or(CommandError::ClipNotFound(id))?;
            if !targets.iter().any(|(_, t)| *t == id) {
                targets.push((project.tracks[track].id == project.main_track, id));
            }
        }
        targets.sort_by_key(|(on_main, _)| *on_main);
        Ok(targets.into_iter().map(|(_, id)| id).collect())
    }
}

impl Command for TrimToPlayhead {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::TrimToPlayhead
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let edge = match self.side {
            Side::Left => Edge::Start,
            Side::Right => Edge::End,
        };
        let mut next = project.clone();
        let mut done: HashSet<ClipId> = HashSet::new();
        for id in self.targets(project)? {
            let Some((_, clip)) = next.clip(id) else {
                continue;
            };
            if done.contains(&id) || !self.crosses(clip) {
                continue;
            }
            done.extend(expand_links(&next, &[id])?);
            let at = self.at;
            self.trims
                .entry(id)
                .or_insert_with(|| TrimClip::new(id, edge, at, true))
                .apply(&mut next)?;
        }
        if next.tracks == project.tracks {
            return Err(CommandError::NothingToEdit);
        }
        self.previous = Some(std::mem::replace(&mut project.tracks, next.tracks));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(tracks) = self.previous.take() {
            project.tracks = tracks;
        }
    }
}

/// Turns the magnetic main track on or off (`setMagnetic`, design §4.17).
///
/// Turning it on repacks the main track (its clips close up from 0 in order) and the clips on
/// other tracks follow (§3.7), all in one history entry. It fails with
/// [`CommandError::TrackLocked`] when the main track is locked and the repack would move a clip.
/// Turning it off only clears the flag. Setting the value it already has changes nothing.
#[derive(Debug)]
pub struct SetMagnetic {
    on: bool,
    track_ids: IdPool<TrackId>,
    previous: Option<(Vec<Track>, bool)>,
}

impl SetMagnetic {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            track_ids: IdPool::default(),
            previous: None,
        }
    }
}

impl Command for SetMagnetic {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SetMagnetic
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let mut next = project.clone();
        next.magnetic = self.on;
        if self.on && !project.magnetic {
            repack_main(&mut next);
            if main_changed(project, &next) {
                let main = project.main().ok_or(CommandError::MainTrack)?;
                if main.locked {
                    return Err(CommandError::TrackLocked(main.id));
                }
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
        }
        next.validate().map_err(CommandError::InvalidResult)?;
        self.previous = Some((
            std::mem::replace(&mut project.tracks, next.tracks),
            std::mem::replace(&mut project.magnetic, next.magnetic),
        ));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some((tracks, magnetic)) = self.previous.take() {
            project.tracks = tracks;
            project.magnetic = magnetic;
        }
    }
}

#[cfg(test)]
mod tests;
