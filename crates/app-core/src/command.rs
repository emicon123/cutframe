//! Edit commands. Every change to a [`Project`] is a command so it can be undone.

use serde::{Deserialize, Serialize};

use crate::media::{MediaAsset, MediaId, MediaProblem, validate_asset};
use crate::model::{CanvasBackground, Project, ProjectSettings, Track, TrackId, TrackKind};
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
    /// One import job: every file it added.
    ImportMedia,
    /// Removing media items from the project (one or several at once).
    RemoveMedia,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandError {
    #[error("track {0:?} does not exist")]
    TrackNotFound(TrackId),
    #[error("name must not be empty")]
    EmptyName,
    #[error("media {0} is already in the project")]
    DuplicateMedia(MediaId),
    #[error("media {0} is not in the project")]
    MediaNotFound(MediaId),
    /// Clips still use the media; remove them first (from task 0260 on, the remove-media batch
    /// does that in the same history entry).
    #[error("media {0} is used by clips")]
    MediaInUse(MediaId),
    #[error("media {id} is invalid: {problem}")]
    InvalidMedia { id: MediaId, problem: MediaProblem },
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
    /// A history jump to a point that is not in the history ([`crate::Document::jump_to`]).
    #[error("history point {index} is out of range (0..={len})")]
    HistoryOutOfRange { index: usize, len: usize },
    /// The main track cannot be removed or moved.
    #[error("the main track cannot be removed or moved")]
    MainTrack,
    /// The edit would change a clip on a locked track.
    #[error("track {0:?} is locked")]
    TrackLocked(TrackId),
    /// The clip would overlap another clip on its track.
    #[error("the clip would overlap another clip")]
    Overlap,
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

/// Inserts a new empty track at `index`, clamped into the section that fits `kind` (video and text:
/// the overlay section above the main track; audio: below it). `None` is the bottom of that
/// section.
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
        let main = project.main_index().unwrap_or(project.tracks.len());
        let section = match self.kind {
            TrackKind::Audio => (main + 1).min(project.tracks.len())..=project.tracks.len(),
            TrackKind::Video | TrackKind::Text => 0..=main,
        };
        let index = self
            .index
            .unwrap_or(*section.end())
            .clamp(*section.start(), *section.end());
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

/// Removes a track with all its clips. The main track cannot be removed
/// ([`CommandError::MainTrack`]).
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
        if self.id == project.main_track {
            return Err(CommandError::MainTrack);
        }
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

/// Appends imported media assets (one import job = one history entry, design §3).
///
/// The ids are fixed when the job builds the command, so redo recreates the same ids. Fails,
/// changing nothing, if an id is already in the project or repeats within `assets`, or if an asset
/// breaks a media invariant.
#[derive(Debug)]
pub struct AddMedia {
    assets: Vec<MediaAsset>,
}

impl AddMedia {
    pub fn new(assets: Vec<MediaAsset>) -> Self {
        Self { assets }
    }

    pub fn ids(&self) -> impl Iterator<Item = MediaId> + '_ {
        self.assets.iter().map(|a| a.id)
    }
}

impl Command for AddMedia {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::ImportMedia
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let mut ids: std::collections::HashSet<MediaId> =
            project.media.iter().map(|m| m.id).collect();
        for asset in &self.assets {
            if !ids.insert(asset.id) {
                return Err(CommandError::DuplicateMedia(asset.id));
            }
            validate_asset(asset).map_err(|problem| CommandError::InvalidMedia {
                id: asset.id,
                problem,
            })?;
        }
        project.media.extend(self.assets.iter().cloned());
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        let ids: std::collections::HashSet<MediaId> = self.ids().collect();
        project.media.retain(|m| !ids.contains(&m.id));
    }
}

/// Removes media assets. Fails with [`CommandError::MediaInUse`] while a clip or the canvas
/// background references one of them, and with [`CommandError::MediaNotFound`] for an id that is not in the project. Revert
/// puts every asset back at its original position.
#[derive(Debug)]
pub struct RemoveMedia {
    ids: Vec<MediaId>,
    /// (index in `project.media` before the removal, asset), ascending by index.
    removed: Vec<(usize, MediaAsset)>,
}

impl RemoveMedia {
    /// Repeated ids count once.
    pub fn new(ids: Vec<MediaId>) -> Self {
        let mut unique = Vec::with_capacity(ids.len());
        for id in ids {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }
        Self {
            ids: unique,
            removed: Vec::new(),
        }
    }
}

/// How many clips reference media `id` (the media panel's "used in timeline" count).
pub fn media_users(project: &Project, id: MediaId) -> usize {
    project.media_users(id)
}

impl Command for RemoveMedia {
    fn label(&self) -> HistoryLabel {
        HistoryLabel::RemoveMedia
    }

    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let mut indices = Vec::with_capacity(self.ids.len());
        for &id in &self.ids {
            let index = project
                .media
                .iter()
                .position(|m| m.id == id)
                .ok_or(CommandError::MediaNotFound(id))?;
            let background = project.background == CanvasBackground::Image { media: id };
            if background || media_users(project, id) > 0 {
                return Err(CommandError::MediaInUse(id));
            }
            indices.push(index);
        }
        indices.sort_unstable();
        self.removed = indices
            .iter()
            .rev()
            .map(|&index| (index, project.media.remove(index)))
            .collect();
        self.removed.reverse();
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) {
        for (index, asset) in std::mem::take(&mut self.removed) {
            project.media.insert(index, asset);
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
        let id = project.tracks[1].id;
        let after = assert_reversible(RemoveTrack::new(id), &project);
        assert!(after.track(id).is_none());
    }

    #[test]
    fn the_main_track_cannot_be_removed() {
        let mut project = Project::new("Test");
        let before = project.clone();
        assert_eq!(
            RemoveTrack::new(project.main_track).apply(&mut project),
            Err(CommandError::MainTrack)
        );
        assert_eq!(project, before);
    }

    #[test]
    fn add_track_clamps_into_the_kind_section() {
        let mut project = Project::new("Test");
        // Tracks: Video 1 (main), Audio 1.
        let cases = [
            (TrackKind::Audio, Some(0), 1),
            (TrackKind::Audio, None, 3),
            (TrackKind::Video, Some(9), 0),
            (TrackKind::Text, None, 1),
            (TrackKind::Video, Some(0), 0),
        ];
        for (kind, index, expected) in cases {
            let command = AddTrack::new(kind, index);
            let id = command.track_id();
            project = assert_reversible(command, &project);
            assert_eq!(
                project.track_index(id),
                Some(expected),
                "{kind:?} {index:?}"
            );
            assert_eq!(project.validate(), Ok(()));
        }
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
            (HistoryLabel::ImportMedia, "importMedia"),
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
        let first = project.tracks[1].id;
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
        project.magnetic = false;
        project.tracks[0].clips.push(crate::model::Clip::color(
            "Odd",
            // Not on the 30 or 60 fps grid.
            crate::time::Time(12_345_677),
            crate::time::Time(98_765_431),
            crate::model::Rgba8::BLACK,
        ));
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
    fn add_media_is_reversible_and_keeps_ids_on_redo() {
        use crate::media::samples;
        let mut project = Project::new("Test");
        project.media.push(samples::image(9));
        let assets = vec![samples::video(1), samples::audio(2)];
        let ids: Vec<_> = assets.iter().map(|a| a.id).collect();
        let command = AddMedia::new(assets);
        assert_eq!(command.label(), HistoryLabel::ImportMedia);
        let after = assert_reversible(command, &project);
        let after_ids: Vec<_> = after.media.iter().map(|m| m.id).collect();
        assert_eq!(after_ids, [vec![project.media[0].id], ids].concat());
    }

    #[test]
    fn add_media_rejects_duplicate_ids_and_changes_nothing() {
        use crate::media::samples;
        let mut project = Project::new("Test");
        let existing = samples::video(1);
        project.media.push(existing.clone());
        let original = project.clone();

        let mut again = existing.clone();
        again.path.absolute = "C:\\elsewhere.mp4".into();
        let mut edited = project.clone();
        assert_eq!(
            AddMedia::new(vec![samples::audio(2), again]).apply(&mut edited),
            Err(CommandError::DuplicateMedia(existing.id))
        );

        let twice = samples::audio(3);
        let mut edited = project.clone();
        let err = AddMedia::new(vec![twice.clone(), twice.clone()]).apply(&mut edited);
        assert_eq!(err, Err(CommandError::DuplicateMedia(twice.id)));

        // `Document::execute` applies to a draft, so a failed apply leaves the project untouched.
        let mut doc = crate::Document::new(project);
        assert!(
            doc.execute(Box::new(AddMedia::new(vec![twice.clone(), twice])))
                .is_err()
        );
        assert_eq!(*doc.project(), original);
        assert!(!doc.can_undo());
    }

    #[test]
    fn add_media_rejects_invalid_assets() {
        use crate::media::samples;
        let mut project = Project::new("Test");
        let mut broken = samples::video(1);
        broken.fingerprint.hash = "nope".into();
        assert_eq!(
            AddMedia::new(vec![broken.clone()]).apply(&mut project),
            Err(CommandError::InvalidMedia {
                id: broken.id,
                problem: MediaProblem::InvalidHash
            })
        );
    }

    #[test]
    fn remove_media_is_reversible_and_restores_positions() {
        use crate::media::samples;
        let mut project = Project::new("Test");
        project.media = vec![
            samples::video(1),
            samples::audio(2),
            samples::image(3),
            samples::video(4),
        ];
        let ids = [project.media[3].id, project.media[1].id];
        let command = RemoveMedia::new(vec![ids[0], ids[1], ids[0]]);
        assert_eq!(command.label(), HistoryLabel::RemoveMedia);
        let after = assert_reversible(command, &project);
        let left: Vec<_> = after.media.iter().map(|m| m.id).collect();
        assert_eq!(left, [project.media[0].id, project.media[2].id]);
    }

    #[test]
    fn remove_media_rejects_unknown_ids_and_changes_nothing() {
        use crate::media::samples;
        let mut project = Project::new("Test");
        project.media = vec![samples::video(1), samples::audio(2)];
        let unknown = MediaId::new();
        let mut doc = crate::Document::new(project.clone());
        assert_eq!(
            doc.execute(Box::new(RemoveMedia::new(vec![
                project.media[0].id,
                unknown
            ]))),
            Err(CommandError::MediaNotFound(unknown))
        );
        assert_eq!(*doc.project(), project);
        assert!(!doc.can_undo());
    }

    #[test]
    fn media_used_by_clips_or_the_background_cannot_be_removed() {
        use crate::media::samples;
        use crate::model::{Clip, ClipSource, Rgba8};
        use crate::time::Time;

        let mut project = Project::new("Test");
        project.media = vec![samples::video(1), samples::image(2), samples::audio(3)];
        let (video, image, song) = (
            project.media[0].id,
            project.media[1].id,
            project.media[2].id,
        );
        let clip = |media| Clip {
            source: ClipSource::Media {
                media,
                source_in: Time::ZERO,
            },
            ..Clip::color("C", Time::ZERO, Time(10), Rgba8::BLACK)
        };
        project.tracks[0].clips = vec![
            clip(video),
            Clip {
                start: Time(10),
                ..clip(video)
            },
        ];
        project.background = CanvasBackground::Image { media: image };
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(media_users(&project, video), 2);
        assert_eq!(
            media_users(&project, image),
            0,
            "the background is not a clip"
        );
        assert_eq!(media_users(&project, song), 0);

        for id in [video, image] {
            let mut doc = crate::Document::new(project.clone());
            assert_eq!(
                doc.execute(Box::new(RemoveMedia::new(vec![song, id]))),
                Err(CommandError::MediaInUse(id))
            );
            assert_eq!(*doc.project(), project);
        }
        let after = assert_reversible(RemoveMedia::new(vec![song]), &project);
        assert_eq!(after.media.len(), 2);
    }

    #[test]
    fn rename_project_is_reversible() {
        let project = Project::new("Test");
        let after = assert_reversible(RenameProject::new("Holiday"), &project);
        assert_eq!(after.name, "Holiday");
    }

    mod props {
        use proptest::prelude::*;

        use super::*;
        use crate::Document;
        use crate::media::{samples, validate_media};

        #[derive(Debug, Clone)]
        enum Op {
            /// Import this many assets; `with_duplicate` repeats the first id, so the import fails.
            Import {
                count: usize,
                with_duplicate: bool,
            },
            /// Remove the assets at these positions (modulo the media count); `with_unknown` adds an
            /// id that is not in the project, so the removal fails.
            Remove {
                picks: Vec<usize>,
                with_unknown: bool,
            },
            Undo,
            Redo,
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                3 => (0usize..4, any::<bool>())
                    .prop_map(|(count, with_duplicate)| Op::Import { count, with_duplicate }),
                2 => (prop::collection::vec(0usize..8, 1..4), prop::bool::weighted(0.2))
                    .prop_map(|(picks, with_unknown)| Op::Remove { picks, with_unknown }),
                2 => Just(Op::Undo),
                1 => Just(Op::Redo),
            ]
        }

        proptest! {
            /// Random imports and removals with undo/redo keep the media invariants, never lose or reorder
            /// assets, and undoing everything restores the start state.
            #[test]
            fn imports_and_removals_with_undo_redo_keep_media_valid(ops in prop::collection::vec(op(), 0..40)) {
                let start = Project::new("Test");
                let mut doc = Document::new(start.clone());
                let mut seed = 0u8;
                for op in ops {
                    let before = doc.snapshot();
                    match op {
                        Op::Import { count, with_duplicate } => {
                            let mut assets: Vec<_> = (0..count)
                                .map(|i| {
                                    seed = seed.wrapping_add(1);
                                    match i % 3 {
                                        0 => samples::video(seed),
                                        1 => samples::audio(seed),
                                        _ => samples::image(seed),
                                    }
                                })
                                .collect();
                            if with_duplicate && let Some(first) = assets.first().cloned() {
                                assets.push(first);
                            }
                            let result = doc.execute(Box::new(AddMedia::new(assets.clone())));
                            if with_duplicate && count > 0 {
                                prop_assert!(result.is_err());
                                prop_assert_eq!(&doc.snapshot(), &before);
                            } else {
                                prop_assert!(result.is_ok());
                                prop_assert_eq!(&doc.project().media[before.media.len()..], &assets[..]);
                            }
                        }
                        Op::Remove { picks, with_unknown } => {
                            if before.media.is_empty() && !with_unknown {
                                continue;
                            }
                            let mut ids: Vec<MediaId> = if before.media.is_empty() {
                                Vec::new()
                            } else {
                                picks.iter().map(|p| before.media[p % before.media.len()].id).collect()
                            };
                            if with_unknown {
                                ids.push(MediaId::new());
                            }
                            let result = doc.execute(Box::new(RemoveMedia::new(ids.clone())));
                            if with_unknown {
                                prop_assert!(result.is_err());
                                prop_assert_eq!(&doc.snapshot(), &before);
                            } else {
                                prop_assert!(result.is_ok());
                                let kept: Vec<_> = before.media.iter().filter(|m| !ids.contains(&m.id)).cloned().collect();
                                prop_assert_eq!(&doc.project().media, &kept, "the others keep their order");
                            }
                        }
                        Op::Undo => { doc.undo(); }
                        Op::Redo => { doc.redo(); }
                    }
                    prop_assert_eq!(validate_media(&doc.project().media), Ok(()));
                }
                while doc.undo() {}
                prop_assert_eq!(doc.project(), &start);
            }
        }
    }
}
