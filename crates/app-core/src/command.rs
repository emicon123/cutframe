//! Edit commands. Every change to a [`Project`] is a command so it can be undone.

use serde::{Deserialize, Serialize};

use crate::model::{Project, ProjectSettings, Track, TrackId, TrackKind};
use crate::validate::ValidationError;

/// Stable key naming an undo/redo history entry, e.g. for an "Undo Add Track" menu item.
///
/// This is not display text: the UI translates the key (ADR-0003 §5, ADR-0006 §5). Keys are API,
/// so a variant is never renamed or reused for a different action; add a new one instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum HistoryLabel {
    AddTrack,
    RemoveTrack,
    RenameTrack,
    RenameProject,
    SetProjectSettings,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandError {
    #[error("track {0:?} does not exist")]
    TrackNotFound(TrackId),
    #[error("name must not be empty")]
    EmptyName,
    /// A gesture is open; commit or cancel it before running another edit (ADR-0006 §3).
    #[error("a gesture is in progress")]
    GestureInProgress,
    /// A gesture update or commit was a different kind of edit than the open gesture.
    #[error("gesture {open:?} is open, got {got:?}")]
    GestureMismatch {
        open: HistoryLabel,
        got: HistoryLabel,
    },
    /// Project settings outside the supported ranges ([`ProjectSettings::validate`]).
    #[error("invalid project settings: {0}")]
    InvalidSettings(ValidationError),
}

/// A reversible edit.
///
/// `apply` may record whatever it needs (e.g. the removed track) so that `revert` restores the
/// project exactly. `revert` is only called after a successful `apply` on the same project state,
/// so it cannot fail.
pub trait Command: Send + Sync + std::fmt::Debug {
    /// Key of the history entry this command creates. The UI turns it into text.
    fn label(&self) -> HistoryLabel;
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError>;
    fn revert(&mut self, project: &mut Project);
}

/// Several commands recorded as one history entry, e.g. "delete selection" (ADR-0006 §1).
///
/// Children are applied in order and reverted in reverse. If a child fails, the children already
/// applied are reverted and the batch returns that child's error, leaving the project unchanged.
#[derive(Debug)]
pub struct Batch {
    label: HistoryLabel,
    commands: Vec<Box<dyn Command>>,
}

impl Batch {
    pub fn new(label: HistoryLabel, commands: Vec<Box<dyn Command>>) -> Self {
        Self { label, commands }
    }
}

impl Command for Batch {
    fn label(&self) -> HistoryLabel {
        self.label
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        for applied in 0..self.commands.len() {
            if let Err(err) = self.commands[applied].apply(project) {
                for command in self.commands[..applied].iter_mut().rev() {
                    command.revert(project);
                }
                return Err(err);
            }
        }
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        for command in self.commands.iter_mut().rev() {
            command.revert(project);
        }
    }
}

/// Inserts a new empty track at `index` (or at the bottom).
#[derive(Debug)]
pub struct AddTrack {
    id: TrackId,
    kind: TrackKind,
    index: Option<usize>,
    inserted_at: Option<usize>,
}

impl AddTrack {
    /// The track id is fixed here, so the caller knows it before `apply` and redo recreates the
    /// same id (later commands in the redo stack may refer to it).
    pub fn new(kind: TrackKind, index: Option<usize>) -> Self {
        Self {
            id: TrackId::new(),
            kind,
            index,
            inserted_at: None,
        }
    }

    pub fn track_id(&self) -> TrackId {
        self.id
    }
}

impl Command for AddTrack {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::AddTrack
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let mut track = Track::new(self.kind, project.next_track_name(self.kind));
        track.id = self.id;
        let index = self
            .index
            .unwrap_or(project.tracks.len())
            .min(project.tracks.len());
        project.tracks.insert(index, track);
        self.inserted_at = Some(index);
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(index) = self.inserted_at.take() {
            project.tracks.remove(index);
        }
    }
}

/// Removes a track with all its clips.
#[derive(Debug)]
pub struct RemoveTrack {
    id: TrackId,
    removed: Option<(usize, Track)>,
}

impl RemoveTrack {
    pub fn new(id: TrackId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveTrack {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::RemoveTrack
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let index = project
            .track_index(self.id)
            .ok_or(CommandError::TrackNotFound(self.id))?;
        self.removed = Some((index, project.tracks.remove(index)));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some((index, track)) = self.removed.take() {
            project.tracks.insert(index, track);
        }
    }
}

/// Renames a track.
#[derive(Debug)]
pub struct RenameTrack {
    id: TrackId,
    name: String,
    previous: Option<String>,
}

impl RenameTrack {
    pub fn new(id: TrackId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            previous: None,
        }
    }
}

impl Command for RenameTrack {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::RenameTrack
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(CommandError::EmptyName);
        }
        let index = project
            .track_index(self.id)
            .ok_or(CommandError::TrackNotFound(self.id))?;
        self.previous = Some(std::mem::replace(
            &mut project.tracks[index].name,
            name.to_owned(),
        ));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let (Some(previous), Some(index)) = (self.previous.take(), project.track_index(self.id))
        {
            project.tracks[index].name = previous;
        }
    }
}

/// Renames the project.
#[derive(Debug)]
pub struct RenameProject {
    name: String,
    previous: Option<String>,
}

impl RenameProject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            previous: None,
        }
    }
}

impl Command for RenameProject {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::RenameProject
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(CommandError::EmptyName);
        }
        self.previous = Some(std::mem::replace(&mut project.name, name.to_owned()));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(previous) = self.previous.take() {
            project.name = previous;
        }
    }
}

/// Replaces the project settings (canvas size, frame rate, sample rate) as one history entry.
///
/// Clips are never touched: their flick times stay bit-exact, and they need not lie on the new
/// frame grid. The aspect ratio is derived from the canvas size, not stored.
#[derive(Debug)]
pub struct SetProjectSettings {
    new: ProjectSettings,
    old: Option<ProjectSettings>,
}

impl SetProjectSettings {
    pub fn new(settings: ProjectSettings) -> Self {
        Self {
            new: settings,
            old: None,
        }
    }
}

impl Command for SetProjectSettings {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::SetProjectSettings
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.new.validate().map_err(CommandError::InvalidSettings)?;
        self.old = Some(std::mem::replace(&mut project.settings, self.new));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        if let Some(old) = self.old.take() {
            project.settings = old;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_reversible(mut command: impl Command, project: &Project) -> Project {
        let mut edited = project.clone();
        command.apply(&mut edited).expect("apply");
        let after_apply = edited.clone();
        command.revert(&mut edited);
        assert_eq!(
            &edited,
            project,
            "{:?} revert must restore the original",
            command.label()
        );
        command.apply(&mut edited).expect("re-apply");
        assert_eq!(
            edited,
            after_apply,
            "{:?} re-apply must reproduce the edit",
            command.label()
        );
        after_apply
    }

    #[test]
    fn add_track_is_reversible_and_keeps_id_on_redo() {
        let project = Project::new("Test");
        let command = AddTrack::new(TrackKind::Video, Some(0));
        let id = command.track_id();
        let after = assert_reversible(command, &project);
        assert_eq!(after.tracks.len(), 3);
        assert_eq!(after.tracks[0].id, id);
        assert_eq!(after.tracks[0].name, "Video 2");
    }

    #[test]
    fn remove_track_is_reversible() {
        let project = Project::new("Test");
        let id = project.tracks[0].id;
        let after = assert_reversible(RemoveTrack::new(id), &project);
        assert!(after.track(id).is_none());
    }

    #[test]
    fn remove_missing_track_fails() {
        let mut project = Project::new("Test");
        let missing = TrackId::new();
        assert_eq!(
            RemoveTrack::new(missing).apply(&mut project),
            Err(CommandError::TrackNotFound(missing))
        );
    }

    #[test]
    fn rename_track_is_reversible_and_trims() {
        let project = Project::new("Test");
        let id = project.tracks[1].id;
        let after = assert_reversible(RenameTrack::new(id, "  Music "), &project);
        assert_eq!(after.track(id).map(|t| t.name.as_str()), Some("Music"));
    }

    #[test]
    fn rename_rejects_empty_names() {
        let mut project = Project::new("Test");
        assert_eq!(
            RenameProject::new("   ").apply(&mut project),
            Err(CommandError::EmptyName)
        );
    }

    #[test]
    fn commands_report_their_history_label() {
        let project = Project::new("Test");
        let id = project.tracks[0].id;
        assert_eq!(
            AddTrack::new(TrackKind::Video, None).label(),
            HistoryLabel::AddTrack
        );
        assert_eq!(RemoveTrack::new(id).label(), HistoryLabel::RemoveTrack);
        assert_eq!(RenameTrack::new(id, "A").label(), HistoryLabel::RenameTrack);
        assert_eq!(RenameProject::new("A").label(), HistoryLabel::RenameProject);
        assert_eq!(
            SetProjectSettings::new(ProjectSettings::default()).label(),
            HistoryLabel::SetProjectSettings
        );
    }

    /// The serialized keys are what the UI translates; renaming one breaks its translations.
    #[test]
    fn history_labels_serialize_as_stable_camel_case_keys() {
        let keys = [
            (HistoryLabel::AddTrack, "addTrack"),
            (HistoryLabel::RemoveTrack, "removeTrack"),
            (HistoryLabel::RenameTrack, "renameTrack"),
            (HistoryLabel::RenameProject, "renameProject"),
            (HistoryLabel::SetProjectSettings, "setProjectSettings"),
        ];
        for (label, key) in keys {
            assert_eq!(serde_json::to_value(label).unwrap(), key);
        }
    }

    #[test]
    fn batch_is_reversible_and_keeps_its_label() {
        let project = Project::new("Test");
        let id = project.tracks[0].id;
        let batch = Batch::new(
            HistoryLabel::RenameTrack,
            vec![
                Box::new(RenameTrack::new(id, "First")),
                Box::new(RenameTrack::new(id, "Second")),
                Box::new(RenameProject::new("Renamed")),
            ],
        );
        assert_eq!(batch.label(), HistoryLabel::RenameTrack);
        let after = assert_reversible(batch, &project);
        assert_eq!(after.track(id).map(|t| t.name.as_str()), Some("Second"));
        assert_eq!(after.name, "Renamed");
    }

    #[test]
    fn batch_rolls_back_applied_children_when_one_fails() {
        let project = Project::new("Test");
        let first = project.tracks[0].id;
        let missing = TrackId::new();
        let mut batch = Batch::new(
            HistoryLabel::RemoveTrack,
            vec![
                Box::new(RenameProject::new("Renamed")),
                Box::new(RemoveTrack::new(first)),
                Box::new(RemoveTrack::new(missing)),
                Box::new(RenameProject::new("Never applied")),
            ],
        );
        let mut edited = project.clone();
        assert_eq!(
            batch.apply(&mut edited),
            Err(CommandError::TrackNotFound(missing))
        );
        assert_eq!(edited, project);
    }

    fn vertical_60fps() -> ProjectSettings {
        ProjectSettings {
            width: 1080,
            height: 1920,
            frame_rate: crate::time::FrameRate::FPS_60,
            sample_rate: 44_100,
        }
    }

    #[test]
    fn set_project_settings_is_reversible_and_leaves_clips_alone() {
        let mut project = Project::new("Test");
        project.tracks[0].clips.push(crate::model::Clip {
            id: crate::model::ClipId::new(),
            name: "Odd".into(),
            // Not on the 30 or 60 fps grid.
            start: crate::time::Time(12_345_677),
            duration: crate::time::Time(98_765_431),
        });
        let after = assert_reversible(SetProjectSettings::new(vertical_60fps()), &project);
        assert_eq!(after.settings, vertical_60fps());
        assert_eq!(after.tracks, project.tracks, "clips are bit-identical");
    }

    #[test]
    fn set_project_settings_rejects_invalid_settings() {
        let mut project = Project::new("Test");
        let before = project.clone();
        let invalid = ProjectSettings {
            width: 1921,
            ..ProjectSettings::default()
        };
        assert_eq!(
            SetProjectSettings::new(invalid).apply(&mut project),
            Err(CommandError::InvalidSettings(ValidationError::CanvasSize {
                width: 1921,
                height: 1080
            }))
        );
        assert_eq!(project, before);
    }

    #[test]
    fn rename_project_is_reversible() {
        let project = Project::new("Test");
        let after = assert_reversible(RenameProject::new("Holiday"), &project);
        assert_eq!(after.name, "Holiday");
    }
}
