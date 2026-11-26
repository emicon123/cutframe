//! Track controls: mute, hide, lock and reorder (design `timeline-editing.md` §3.3, §4.15,
//! §4.16).

use serde::{Deserialize, Serialize};

use super::{Command, CommandError, HistoryLabel};
use crate::model::{Project, Track, TrackId, TrackKind};

/// A track's on/off switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum TrackFlag {
    /// No audio from the track in preview and export. Editing is unaffected.
    Muted,
    /// No visuals from the track in preview and export. Editing is unaffected.
    Hidden,
    /// Edits never change the track's clips, and its clips never follow the main track.
    Locked,
}

/// Turns one flag of a track on or off (`setTrackMuted`, `setTrackHidden`, `setTrackLocked`).
/// Works on every track, locked or not, including the main track. Setting the value the flag
/// already has changes nothing; [`SetTrackFlag::changes`] lets the caller skip the history entry.
#[derive(Debug)]
pub struct SetTrackFlag {
    track: TrackId,
    flag: TrackFlag,
    value: bool,
    previous: Option<bool>,
}

impl SetTrackFlag {
    pub fn new(track: TrackId, flag: TrackFlag, value: bool) -> Self {
        Self {
            track,
            flag,
            value,
            previous: None,
        }
    }

    /// Whether applying would change `project`. Fails for an unknown track.
    pub fn changes(&self, project: &Project) -> Result<bool, CommandError> {
        let index = project
            .track_index(self.track)
            .ok_or(CommandError::TrackNotFound(self.track))?;
        let track = &project.tracks[index];
        let current = match self.flag {
            TrackFlag::Muted => track.muted,
            TrackFlag::Hidden => track.hidden,
            TrackFlag::Locked => track.locked,
        };
        Ok(current != self.value)
    }

    fn slot<'a>(&self, track: &'a mut Track) -> &'a mut bool {
        match self.flag {
            TrackFlag::Muted => &mut track.muted,
            TrackFlag::Hidden => &mut track.hidden,
            TrackFlag::Locked => &mut track.locked,
        }
    }
}

impl Command for SetTrackFlag {
    fn label(&self) -> HistoryLabel {
        match self.flag {
            TrackFlag::Muted => HistoryLabel::SetTrackMuted,
            TrackFlag::Hidden => HistoryLabel::SetTrackHidden,
            TrackFlag::Locked => HistoryLabel::SetTrackLocked,
        }
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let index = project
            .track_index(self.track)
            .ok_or(CommandError::TrackNotFound(self.track))?;
        let value = self.value;
        let slot = self.slot(&mut project.tracks[index]);
        self.previous = Some(std::mem::replace(slot, value));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let (Some(previous), Some(index)) =
            (self.previous.take(), project.track_index(self.track))
        {
            *self.slot(&mut project.tracks[index]) = previous;
        }
    }
}

/// Moves a track to `index` (top = 0, the index it has afterwards), clamped into its own
/// section: overlay tracks (video, text) stay above the main track and audio tracks below it
/// (`moveTrack`). Clips move with their track and keep their times. The main track cannot move
/// ([`CommandError::MainTrack`]). Lock, mute and hide do not stop a reorder.
#[derive(Debug)]
pub struct MoveTrack {
    track: TrackId,
    index: usize,
    previous: Option<usize>,
}

impl MoveTrack {
    pub fn new(track: TrackId, index: usize) -> Self {
        Self {
            track,
            index,
            previous: None,
        }
    }

    /// The track's current index and the index it would move to.
    fn target(&self, project: &Project) -> Result<(usize, usize), CommandError> {
        let from = project
            .track_index(self.track)
            .ok_or(CommandError::TrackNotFound(self.track))?;
        if self.track == project.main_track {
            return Err(CommandError::MainTrack);
        }
        let main = project.main_index().ok_or(CommandError::MainTrack)?;
        // Indices in the list without the moving track.
        let main = if from < main { main - 1 } else { main };
        let last = project.tracks.len() - 1;
        let (first, last) = match project.tracks[from].kind {
            TrackKind::Audio => (main + 1, last),
            TrackKind::Video | TrackKind::Text => (0, main),
        };
        Ok((from, self.index.clamp(first, last)))
    }

    /// Whether applying would change `project` (the clamped index differs from the current
    /// one). Fails as `apply` does.
    pub fn changes(&self, project: &Project) -> Result<bool, CommandError> {
        let (from, to) = self.target(project)?;
        Ok(from != to)
    }
}

impl Command for MoveTrack {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::MoveTrack
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let (from, to) = self.target(project)?;
        let track = project.tracks.remove(from);
        project.tracks.insert(to, track);
        self.previous = Some(from);
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let (Some(from), Some(to)) = (self.previous.take(), project.track_index(self.track)) {
            let track = project.tracks.remove(to);
            project.tracks.insert(from, track);
        }
    }
}

#[cfg(test)]
mod tests;
