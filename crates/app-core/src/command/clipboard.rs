//! The clip clipboard and pasting (design `timeline-editing.md` §4.8, §6.3).
//!
//! The clipboard itself lives in the shell (`AppState`), one per app session, and is never saved:
//! media ids belong to a project. This module holds its content type and the paste command that
//! both paste (Ctrl+V) and duplicate (Ctrl+D) use.

use std::collections::{HashMap, HashSet};

use super::{Command, CommandError, HistoryLabel};
use crate::model::{Clip, ClipId, LinkId, Project, Track, TrackId, TrackKind};
use crate::time::Time;
use crate::timeline::{
    Follow, IdPool, expand_links, follow_main, is_free, nearest_boundary, normalize_links,
    place_clip, repack_main, section_index,
};

/// Copied clips: deep copies, with their times relative to the earliest copied start.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardContent {
    /// In track order, then clip order.
    pub items: Vec<ClipboardItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardItem {
    /// A deep copy. Its id and link only regroup the copies; pasting gives new ones.
    pub clip: Clip,
    /// The track it was copied from.
    pub track: TrackId,
    /// That track's kind, for the new track a paste falls back to.
    pub kind: TrackKind,
    pub on_main: bool,
    /// `clip.start` minus the earliest copied start.
    pub offset: Time,
}

impl ClipboardContent {
    /// Copies `clips` and their linked members. Fails with [`CommandError::NothingToEdit`] for
    /// an empty list and [`CommandError::ClipNotFound`] for an unknown id.
    pub fn copy(project: &Project, clips: &[ClipId]) -> Result<Self, CommandError> {
        if clips.is_empty() {
            return Err(CommandError::NothingToEdit);
        }
        let ids: HashSet<ClipId> = expand_links(project, clips)?.into_iter().collect();
        let first = project
            .clips()
            .filter(|c| ids.contains(&c.id))
            .map(|c| c.start)
            .min()
            .unwrap_or(Time::ZERO);
        let items = project
            .tracks
            .iter()
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|c| ids.contains(&c.id))
                    .map(move |c| ClipboardItem {
                        clip: c.clone(),
                        track: track.id,
                        kind: track.kind,
                        on_main: track.id == project.main_track,
                        offset: c.start - first,
                    })
            })
            .collect();
        Ok(Self { items })
    }
}

/// Pastes clipboard content at `at` (`pasteClips`, `duplicateClips`, design §6.3).
///
/// - **Main-track items** go onto the main track. On the magnetic main track they are inserted
///   as one block at the clip boundary nearest `at`, the later main clips ripple and the clips on
///   other tracks follow (§3.7); every other item is then placed relative to where the block
///   landed, so copied groups stay in sync. On a non-magnetic main track each goes to
///   `at + offset` if that range is free.
/// - **Other items** go to `at + offset` (never before 0) on their original track when it still
///   exists, is unlocked and the range is free.
/// - Everything else goes onto a new track of its kind, created directly above the main track
///   for video and text clips and directly below it for audio; one new track takes several items
///   when they fit. A locked track is never a paste target.
///
/// Every pasted clip gets a new id ([`PasteClips::clip_ids`], fixed at construction so redo
/// recreates them) and each copied link group one new link. Fails with
/// [`CommandError::NothingToEdit`] for empty content and [`CommandError::MediaNotFound`] when
/// copied media was removed from the project since.
#[derive(Debug)]
pub struct PasteClips {
    content: ClipboardContent,
    at: Time,
    label: HistoryLabel,
    ids: Vec<ClipId>,
    links: HashMap<LinkId, LinkId>,
    track_ids: IdPool<TrackId>,
    previous: Option<Vec<Track>>,
}

impl PasteClips {
    pub fn new(content: ClipboardContent, at: Time) -> Self {
        let ids = content.items.iter().map(|_| ClipId::new()).collect();
        let links = content
            .items
            .iter()
            .filter_map(|i| i.clip.link)
            .map(|l| (l, LinkId::new()))
            .collect();
        Self {
            content,
            at,
            label: HistoryLabel::PasteClips,
            ids,
            links,
            track_ids: IdPool::default(),
            previous: None,
        }
    }

    /// Duplicates `clips` and their linked members (Ctrl+D): a paste of their copy at the
    /// latest end among them, without touching the clipboard.
    pub fn duplicate(project: &Project, clips: &[ClipId]) -> Result<Self, CommandError> {
        let content = ClipboardContent::copy(project, clips)?;
        let ids: HashSet<ClipId> = content.items.iter().map(|i| i.clip.id).collect();
        let end = project
            .clips()
            .filter(|c| ids.contains(&c.id))
            .map(Clip::end)
            .max()
            .unwrap_or(Time::ZERO);
        Ok(Self {
            label: HistoryLabel::DuplicateClips,
            ..Self::new(content, end)
        })
    }

    /// The ids of the pasted clips, one per item in clipboard order.
    pub fn clip_ids(&self) -> &[ClipId] {
        &self.ids
    }

    fn pasted(&mut self, project: &Project) -> Result<Project, CommandError> {
        if self.content.items.is_empty() {
            return Err(CommandError::NothingToEdit);
        }
        let mut clips: Vec<(&ClipboardItem, Clip)> = Vec::with_capacity(self.ids.len());
        for (item, &id) in self.content.items.iter().zip(&self.ids) {
            if let Some(media) = item.clip.source.media()
                && project.media_asset(media).is_none()
            {
                return Err(CommandError::MediaNotFound(media));
            }
            let mut clip = item.clip.clone();
            clip.id = id;
            clip.link = clip.link.map(|l| self.links[&l]);
            clips.push((item, clip));
        }

        let mut next = project.clone();
        let main_index = next.main_index().ok_or(CommandError::MainTrack)?;
        let main_locked = next.tracks[main_index].locked;
        let (main_items, other_items): (Vec<_>, Vec<_>) = clips
            .into_iter()
            .partition(|(item, _)| item.on_main && !main_locked);
        let mut at = self.at;
        // Items that go to a new track: (kind, clip with its start set).
        let mut homeless: Vec<(TrackKind, Clip)> = Vec::new();

        if !main_items.is_empty() && next.magnetic {
            let track = &mut next.tracks[main_index];
            let boundary = nearest_boundary(&track.clips, self.at);
            let first = main_items
                .iter()
                .map(|(item, _)| item.offset)
                .min()
                .unwrap_or(Time::ZERO);
            at = boundary - first;
            let mut block: Vec<Clip> = main_items.into_iter().map(|(_, c)| c).collect();
            block.sort_by_key(|c| c.start);
            let index = track.clips.partition_point(|c| c.start < boundary);
            track.clips.splice(index..index, block);
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
        } else {
            for (item, mut clip) in main_items {
                clip.start = Time((at + item.offset).flicks().max(0));
                let track = &mut next.tracks[main_index];
                if is_free(track, clip.start, clip.end(), &[]) {
                    place_clip(track, clip)?;
                } else {
                    homeless.push((item.kind, clip));
                }
            }
        }

        let mut others = other_items;
        others.sort_by_key(|(item, _)| item.offset);
        for (item, mut clip) in others {
            clip.start = Time((at + item.offset).flicks().max(0));
            let target = next.tracks.iter_mut().find(|t| {
                t.id == item.track && t.id != project.main_track && fits(t, item.kind, &clip)
            });
            match target {
                Some(track) => {
                    place_clip(track, clip)?;
                }
                None => homeless.push((item.kind, clip)),
            }
        }

        let mut created: Vec<TrackId> = Vec::new();
        for (kind, clip) in homeless {
            let existing = created
                .iter()
                .copied()
                .find(|&id| next.track(id).is_some_and(|t| fits(t, kind, &clip)));
            let id = existing.unwrap_or_else(|| {
                let mut track = Track::new(kind, next.next_track_name(kind));
                track.id = self.track_ids.take();
                let id = track.id;
                let main = next.main_index().expect("main track exists");
                let index = match kind {
                    TrackKind::Audio => main + 1,
                    TrackKind::Video | TrackKind::Text => section_index(&next, kind, None),
                };
                next.tracks.insert(index, track);
                created.push(id);
                id
            });
            let track = next
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .expect("track exists");
            place_clip(track, clip)?;
        }
        normalize_links(&mut next);
        next.validate().map_err(CommandError::InvalidResult)?;
        Ok(next)
    }
}

/// Whether `clip` may be pasted onto `track`: same kind, unlocked, the range free, and no other
/// member of its link group there.
fn fits(track: &Track, kind: TrackKind, clip: &Clip) -> bool {
    track.kind == kind
        && !track.locked
        && is_free(track, clip.start, clip.end(), &[])
        && (clip.link.is_none() || track.clips.iter().all(|c| c.link != clip.link))
}

impl Command for PasteClips {
    fn label(&self) -> HistoryLabel {
        self.label
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.track_ids.rewind();
        let next = self.pasted(project)?;
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
