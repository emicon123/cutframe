//! Shared timeline helpers for edit commands (design `timeline-editing.md` §3.3, §3.5, §3.7, §4).
//!
//! Commands combine these: they change the main track, call [`repack_main`] and [`follow_main`],
//! then [`normalize_links`]. Each helper keeps the §3.4 invariants it is responsible for.

use std::collections::HashMap;

use crate::command::CommandError;
use crate::model::{Clip, ClipId, LinkId, Project, Track, TrackId, TrackKind};
use crate::time::Time;

/// Whether `start..end` is free on `track`, ignoring the clips in `ignore`.
pub fn is_free(track: &Track, start: Time, end: Time, ignore: &[ClipId]) -> bool {
    track
        .clips
        .iter()
        .filter(|c| !ignore.contains(&c.id))
        .all(|c| c.end() <= start || end <= c.start)
}

/// Inserts `clip` into `track`, keeping the clips sorted by start. Fails with
/// [`CommandError::Overlap`], changing nothing, if it would overlap a clip already there.
/// Returns the clip's index in the track.
pub fn place_clip(track: &mut Track, clip: Clip) -> Result<usize, CommandError> {
    if !is_free(track, clip.start, clip.end(), &[]) {
        return Err(CommandError::Overlap);
    }
    let index = track.clips.partition_point(|c| c.start <= clip.start);
    track.clips.insert(index, clip);
    Ok(index)
}

/// Where a new track of `kind` goes for a requested `index`: clamped into the overlay section
/// (video, text: `0..=main`) or the audio section (`main + 1..=len`). `None` is the bottom of the
/// section.
pub fn section_index(project: &Project, kind: TrackKind, index: Option<usize>) -> usize {
    let len = project.tracks.len();
    let main = project.main_index().unwrap_or(len);
    let (first, last) = match kind {
        TrackKind::Audio => ((main + 1).min(len), len),
        TrackKind::Video | TrackKind::Text => (0, main),
    };
    index.unwrap_or(last).clamp(first, last)
}

/// Removes the overlay and audio tracks that held clips in `before` and hold none in `after`
/// (F1.28). Tracks that were already empty, new tracks and the main track are kept. Commands call
/// it on commit only, so the layout does not jump under the pointer during a drag.
pub fn remove_emptied_tracks(before: &Project, after: &mut Project) {
    let had_clips: std::collections::HashSet<TrackId> = before
        .tracks
        .iter()
        .filter(|t| !t.clips.is_empty())
        .map(|t| t.id)
        .collect();
    let main = after.main_track;
    after
        .tracks
        .retain(|t| t.id == main || !t.clips.is_empty() || !had_clips.contains(&t.id));
}

/// While `project.magnetic` is on, recomputes the main track's starts in list order so its clips
/// are contiguous from 0. Does nothing when magnetic is off.
pub fn repack_main(project: &mut Project) {
    if !project.magnetic {
        return;
    }
    let Some(index) = project.main_index() else {
        return;
    };
    let mut end = Time::ZERO;
    for clip in &mut project.tracks[index].clips {
        clip.start = end;
        end = clip.end();
    }
}

/// Clears the link of every group left with fewer than two clips. Run at the end of every command
/// that removes or unlinks clips.
pub fn normalize_links(project: &mut Project) {
    let mut counts: HashMap<LinkId, usize> = HashMap::new();
    for link in project.clips().filter_map(|c| c.link) {
        *counts.entry(link).or_default() += 1;
    }
    for clip in project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if clip.link.is_some_and(|l| counts[&l] < 2) {
            clip.link = None;
        }
    }
}

/// `ids` and every clip linked to one of them (design §3.5), each once, in track order then
/// clip order. Fails with [`CommandError::ClipNotFound`] for an id that is not in the project.
pub fn expand_links(project: &Project, ids: &[ClipId]) -> Result<Vec<ClipId>, CommandError> {
    let mut links = std::collections::HashSet::new();
    for &id in ids {
        let (_, clip) = project.clip(id).ok_or(CommandError::ClipNotFound(id))?;
        links.extend(clip.link);
    }
    Ok(project
        .clips()
        .filter(|c| ids.contains(&c.id) || c.link.is_some_and(|l| links.contains(&l)))
        .map(|c| c.id)
        .collect())
}

/// Keeps a clip's props valid after its duration or content changed: keyframe offsets move by
/// `-content_shift` (a start trim by `d` shifts the content under the clip start by `d`), those that
/// end up outside `0..=duration` are dropped, and fades are shortened to fit the new duration.
/// Tier 1 writes no keyframes, so only the fades ever change; task 0520 adds interpolated boundary
/// keyframes here.
pub fn retime_keyframes(clip: &mut Clip, content_shift: Time) {
    fn retime<T>(animated: &mut crate::props::Animated<T>, shift: Time, duration: Time) {
        for keyframe in &mut animated.keyframes {
            keyframe.time = keyframe.time - shift;
        }
        animated
            .keyframes
            .retain(|k| Time::ZERO <= k.time && k.time <= duration);
    }
    let duration = clip.duration;
    if let Some(visual) = &mut clip.visual {
        retime(&mut visual.position, content_shift, duration);
        retime(&mut visual.scale, content_shift, duration);
        retime(&mut visual.rotation, content_shift, duration);
        retime(&mut visual.opacity, content_shift, duration);
        retime(&mut visual.crop, content_shift, duration);
    }
    if let Some(audio) = &mut clip.audio {
        retime(&mut audio.volume, content_shift, duration);
        audio.fade_in = audio.fade_in.min(duration);
        audio.fade_out = audio.fade_out.min(duration - audio.fade_in);
    }
}

/// Whether the main track's clips differ in position or content range between `before` and
/// `after`: the test for "followers must follow" (§3.7).
pub fn main_changed(before: &Project, after: &Project) -> bool {
    let key = |c: &Clip| (c.id, c.start, c.duration, c.source_in());
    match (before.main(), after.main()) {
        (Some(a), Some(b)) => !a.clips.iter().map(key).eq(b.clips.iter().map(key)),
        _ => false,
    }
}

/// The clip boundary of `clips` (a start or an end, or 0) nearest `at`; ties go to the earlier
/// one. Where an insert into the magnetic main track lands.
pub fn nearest_boundary(clips: &[Clip], at: Time) -> Time {
    clips
        .iter()
        .flat_map(|c| [c.start, c.end()])
        .chain([Time::ZERO])
        .min_by_key(|b| ((*b - at).flicks().unsigned_abs(), *b))
        .unwrap_or(Time::ZERO)
}

/// The ids of the clips in `after` that are not in `before`, in track order then clip order: the
/// clips an edit created (e.g. the right halves of a split).
pub fn added_clips(before: &Project, after: &Project) -> Vec<ClipId> {
    let old: std::collections::HashSet<ClipId> = before.clips().map(|c| c.id).collect();
    after
        .clips()
        .filter(|c| !old.contains(&c.id))
        .map(|c| c.id)
        .collect()
}

/// Moves each clip in `deltas` by its delta and puts it back on its own track. Fails with
/// [`CommandError::Overlap`] (leaving `project` partly changed, so callers work on a copy) when a
/// clip would start before 0 or overlap a clip that stays or was placed before it.
pub fn shift_clips(
    project: &mut Project,
    deltas: &HashMap<ClipId, Time>,
) -> Result<(), CommandError> {
    let mut moving: Vec<(TrackId, Clip)> = Vec::new();
    for track in &mut project.tracks {
        let (out, kept): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut track.clips)
            .into_iter()
            .partition(|c| deltas.contains_key(&c.id));
        track.clips = kept;
        moving.extend(out.into_iter().map(|mut c| {
            c.start = c.start + deltas[&c.id];
            (track.id, c)
        }));
    }
    moving.sort_by_key(|(_, c)| (c.start, c.id));
    for (track, clip) in moving {
        if clip.start < Time::ZERO {
            return Err(CommandError::Overlap);
        }
        place_clip(track_mut(project, track), clip)?;
    }
    Ok(())
}

/// Ids handed out in a fixed order, so a redo of a command recreates the same ids as its first
/// apply. A command keeps one pool and calls [`IdPool::rewind`] at the start of every apply.
#[derive(Debug, Clone, Default)]
pub struct IdPool<T> {
    ids: Vec<T>,
    used: usize,
}

impl<T: Copy + Default> IdPool<T> {
    /// The next id: the one handed out at this point by an earlier apply, or a new one.
    pub fn take(&mut self) -> T {
        if self.used == self.ids.len() {
            self.ids.push(T::default());
        }
        let id = self.ids[self.used];
        self.used += 1;
        id
    }

    /// Starts handing out the same ids again from the first.
    pub fn rewind(&mut self) {
        self.used = 0;
    }
}

/// Where a non-main clip is attached to the main track, derived from positions (§3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The main clip `clip` contains the time. `content` is the source time under it for media
    /// clips, or the offset from the clip's start for colour clips (images have `source_in` 0, so
    /// both are `source_in + offset`).
    Inside { clip: ClipId, content: Time },
    /// The time lies in a gap before main clip `next` (only with magnetic off).
    Gap { next: ClipId },
    /// The time is at or after the end of the main track.
    AfterEnd,
}

/// The anchor of time `t` against the (sorted) main-track clips `main`. `None` when it is empty.
pub fn anchor_of(main: &[Clip], t: Time) -> Option<Anchor> {
    let last = main.last()?;
    if t >= last.end() {
        return Some(Anchor::AfterEnd);
    }
    let next = &main[main.partition_point(|c| c.end() <= t)];
    Some(if next.start <= t {
        Anchor::Inside {
            clip: next.id,
            content: next.source_in() + (t - next.start),
        }
    } else {
        Anchor::Gap { next: next.id }
    })
}

/// Options of [`follow_main`].
#[derive(Debug)]
pub struct Follow<'a> {
    /// Clips the command placed itself (existing clips it moved); they never follow.
    pub moved: &'a [ClipId],
    /// Ripple delete or cut: followers lying entirely inside a removed main clip are deleted with
    /// it instead of moving to the removal point.
    pub delete_inside: bool,
    /// Ids for tracks created when a bumped clip fits nowhere.
    pub track_ids: &'a mut IdPool<TrackId>,
}

/// Moves the clips on unlocked non-main tracks with the main-track edit that turned `before` into
/// `after` (design §3.7). Anchors are resolved in `before`; `after` holds the command's result with
/// the followers still at their old positions.
///
/// - A linked member of a main clip that is still on the main track shifts by that clip's delta.
///   If such a member is on a locked track, this fails with [`CommandError::TrackLocked`] (the
///   project is then partly changed, so the command must discard it, as `Document` does).
/// - A link group off the main track follows as one unit, by the anchor of its earliest member. A
///   group with a member on a locked track, or one the command moved itself, does not follow.
/// - Other clips follow their own anchor. A clip whose anchor was removed goes to the removal
///   point, or is deleted when `delete_inside` is set and it lay entirely inside the removed clip.
/// - Moved clips are placed per track: unmoved clips stay, then moved ones go in order of new
///   start (ties: old start, then id). A clip that collides is bumped to the nearest free track
///   of the same kind in its section (overlays search upward first, audio downward first), or to
///   a new track next to its own.
pub fn follow_main(
    before: &Project,
    after: &mut Project,
    follow: Follow<'_>,
) -> Result<(), CommandError> {
    let (Some(old_main), Some(new_main)) = (before.main(), after.main()) else {
        return Ok(());
    };
    if old_main.clips.is_empty() {
        return Ok(());
    }
    let mapping = MainMapping::new(&old_main.clips, &new_main.clips);
    let main_index = after.main_index().expect("checked above");

    // Old position and track of every clip off the main track.
    let old: HashMap<ClipId, (TrackId, Time)> = before
        .tracks
        .iter()
        .filter(|t| t.id != before.main_track)
        .flat_map(|t| t.clips.iter().map(move |c| (c.id, (t.id, c.start))))
        .collect();
    let main_links: HashMap<LinkId, &Clip> = old_main
        .clips
        .iter()
        .filter_map(|c| c.link.map(|l| (l, c)))
        .collect();

    // Members of each link group off the main track in `after`, and whether every one may follow.
    let mut groups: HashMap<LinkId, (Vec<&Clip>, bool)> = HashMap::new();
    // Per clip: Some(delta) to move, None to delete.
    let mut plan: HashMap<ClipId, Option<Time>> = HashMap::new();
    for (index, track) in after.tracks.iter().enumerate() {
        if index == main_index {
            continue;
        }
        for clip in &track.clips {
            let eligible = !track.locked
                && !follow.moved.contains(&clip.id)
                && old.get(&clip.id).is_some_and(|&(t, _)| t == track.id);
            let old_start = old.get(&clip.id).map(|&(_, s)| s);
            let via_main = clip
                .link
                .and_then(|l| main_links.get(&l))
                .and_then(|m| mapping.surviving(m.id).map(|n| n.start - m.start));
            match (via_main, old_start) {
                (Some(delta), Some(_)) => {
                    if delta != Time::ZERO && !follow.moved.contains(&clip.id) {
                        if track.locked {
                            return Err(CommandError::TrackLocked(track.id));
                        }
                        if eligible {
                            plan.insert(clip.id, Some(delta));
                        }
                    }
                }
                _ => match clip.link {
                    Some(link) => {
                        let group = groups.entry(link).or_insert_with(|| (Vec::new(), true));
                        group.0.push(clip);
                        group.1 &= eligible;
                    }
                    None if eligible => {
                        let start = old_start.expect("eligible clips existed before");
                        let end = start + clip.duration;
                        plan.insert(clip.id, mapping.resolve(start, end, follow.delete_inside));
                    }
                    None => {}
                },
            }
        }
    }
    for (members, eligible) in groups.into_values() {
        if !eligible {
            continue;
        }
        let first = members
            .iter()
            .min_by_key(|c| (c.start, c.id))
            .expect("groups are not empty");
        let resolved = mapping.resolve(first.start, first.end(), false);
        let delete = follow.delete_inside
            && mapping.removed_container(first.start).is_some_and(|a| {
                members
                    .iter()
                    .all(|c| a.start <= c.start && c.end() <= a.end())
            });
        for member in &members {
            let entry = if delete {
                None
            } else {
                // Keep every member at or after 0, so the group stays in sync.
                let lowest = members.iter().map(|c| c.start).min().unwrap_or(Time::ZERO);
                resolved.map(|d| Time(d.flicks().max(-lowest.flicks())))
            };
            plan.insert(member.id, entry);
        }
    }
    plan.retain(|_, change| *change != Some(Time::ZERO));
    if plan.is_empty() {
        return Ok(());
    }

    // Take the moving clips out; deleted ones are dropped.
    let mut moving: Vec<(TrackId, Time, Clip)> = Vec::new();
    let mut deleted = false;
    for track in &mut after.tracks {
        let mut kept = Vec::with_capacity(track.clips.len());
        for clip in std::mem::take(&mut track.clips) {
            match plan.get(&clip.id) {
                None => kept.push(clip),
                Some(None) => deleted = true,
                Some(Some(delta)) => {
                    let old_start = clip.start;
                    let mut clip = clip;
                    clip.start = Time((clip.start + *delta).flicks().max(0));
                    moving.push((track.id, old_start, clip));
                }
            }
        }
        track.clips = kept;
    }
    let key = |(_, old_start, clip): &(TrackId, Time, Clip)| (clip.start, *old_start, clip.id);
    moving.sort_by_key(key);

    // Own track first, so a track's own followers never give way to clips bumped from elsewhere.
    let mut bumped = Vec::new();
    for (track_id, old_start, clip) in moving {
        let track = track_mut(after, track_id);
        if is_free(track, clip.start, clip.end(), &[]) {
            place_clip(track, clip).expect("range is free");
        } else {
            bumped.push((track_id, old_start, clip));
        }
    }
    for (track_id, _, clip) in bumped {
        let target = bump_target(after, track_id, &clip).unwrap_or_else(|| {
            let index = after.track_index(track_id).expect("track exists");
            let kind = after.tracks[index].kind;
            let mut track = Track::new(kind, after.next_track_name(kind));
            track.id = follow.track_ids.take();
            let id = track.id;
            let at = if kind == TrackKind::Audio {
                index + 1
            } else {
                index
            };
            after.tracks.insert(at, track);
            id
        });
        place_clip(track_mut(after, target), clip).expect("bump target is free");
    }
    if deleted {
        normalize_links(after);
    }
    Ok(())
}

fn track_mut(project: &mut Project, id: TrackId) -> &mut Track {
    project
        .tracks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("track exists")
}

/// The nearest unlocked track of the same kind in the same section where `clip` fits and that
/// holds no other member of its link group. Overlays search upward first, audio downward first.
fn bump_target(project: &Project, from: TrackId, clip: &Clip) -> Option<TrackId> {
    let main = project.main_index()?;
    let origin = project.track_index(from)?;
    let kind = project.tracks[origin].kind;
    let (up, down): (Vec<usize>, Vec<usize>) = if kind == TrackKind::Audio {
        (
            (main + 1..origin).rev().collect(),
            (origin + 1..project.tracks.len()).collect(),
        )
    } else {
        ((0..origin).rev().collect(), (origin + 1..main).collect())
    };
    let order = if kind == TrackKind::Audio {
        down.into_iter().chain(up)
    } else {
        up.into_iter().chain(down)
    };
    order
        .map(|i| &project.tracks[i])
        .find(|t| {
            t.kind == kind
                && !t.locked
                && t.id != project.main_track
                && (clip.link.is_none() || t.clips.iter().all(|c| c.link != clip.link))
                && is_free(t, clip.start, clip.end(), &[])
        })
        .map(|t| t.id)
}

/// The main track before and after an edit, for resolving anchors.
struct MainMapping<'a> {
    old: &'a [Clip],
    new: HashMap<ClipId, &'a Clip>,
    old_end: Time,
    new_end: Time,
}

impl<'a> MainMapping<'a> {
    fn new(old: &'a [Clip], new: &'a [Clip]) -> Self {
        Self {
            old,
            new: new.iter().map(|c| (c.id, c)).collect(),
            old_end: old.last().map_or(Time::ZERO, Clip::end),
            new_end: new.iter().map(Clip::end).max().unwrap_or(Time::ZERO),
        }
    }

    /// The clip `id` on the main track after the edit.
    fn surviving(&self, id: ClipId) -> Option<&'a Clip> {
        self.new.get(&id).copied()
    }

    /// The removed main clip that contains `t`, if any.
    fn removed_container(&self, t: Time) -> Option<&'a Clip> {
        match anchor_of(self.old, t)? {
            Anchor::Inside { clip, .. } if self.surviving(clip).is_none() => {
                self.old.iter().find(|c| c.id == clip)
            }
            _ => None,
        }
    }

    /// The change of start of a follower at `start..end`: `Some(delta)`, or `None` to delete it
    /// (only with `delete_inside`).
    fn resolve(&self, start: Time, end: Time, delete_inside: bool) -> Option<Time> {
        let new_start = match anchor_of(self.old, start) {
            None => start,
            Some(Anchor::AfterEnd) => Time((start + self.new_end - self.old_end).flicks().max(0)),
            Some(Anchor::Gap { .. }) => self.shift_point(start),
            Some(Anchor::Inside { clip, content }) => match self.surviving(clip) {
                Some(anchor) => {
                    let offset = (content - anchor.source_in())
                        .flicks()
                        .clamp(0, anchor.duration.flicks());
                    anchor.start + Time(offset)
                }
                None => {
                    let removed = self.old.iter().find(|c| c.id == clip).expect("old clip");
                    if delete_inside && removed.start <= start && end <= removed.end() {
                        return None;
                    }
                    self.shift_point(removed.start)
                }
            },
        };
        Some(new_start - start)
    }

    /// Where a point that is not inside a surviving main clip goes: with the next surviving clip
    /// after it, else with the end of the last surviving clip before it, and never before that
    /// clip's new end (or 0).
    fn shift_point(&self, p: Time) -> Time {
        let surviving = |c: &&Clip| self.new.contains_key(&c.id);
        let prev = self.old.iter().filter(surviving).rfind(|c| c.end() <= p);
        let next = self.old.iter().filter(surviving).find(|c| c.start >= p);
        let shifted = match (next, prev) {
            (Some(n), _) => p + (self.new[&n.id].start - n.start),
            (None, Some(c)) => p + (self.new[&c.id].end() - c.end()),
            (None, None) => p,
        };
        let floor = prev.map_or(Time::ZERO, |c| self.new[&c.id].end());
        shifted.max(floor)
    }
}

#[cfg(test)]
mod tests;
