//! An open project: the current state, its undo/redo history and a revision counter.

use std::sync::Arc;

use crate::command::{Command, CommandError};
use crate::model::Project;

/// Maximum number of undo steps kept in memory.
pub const DEFAULT_HISTORY_LIMIT: usize = 500;

/// The editable document. Owned by one thread; readers get cheap [`Arc`] snapshots.
///
/// Each edit clones the project only if a snapshot is still held elsewhere (copy-on-write via
/// [`Arc::make_mut`]).
#[derive(Debug)]
pub struct Document {
    project: Arc<Project>,
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
    revision: u64,
    history_limit: usize,
}

impl Document {
    pub fn new(project: Project) -> Self {
        Self {
            project: Arc::new(project),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }

    /// The current state. Cheap to clone and safe to send to other threads.
    pub fn snapshot(&self) -> Arc<Project> {
        Arc::clone(&self.project)
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Increases by one on every successful change (execute, undo, redo).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|c| c.label())
    }

    /// Applies `command` and records it. On error, the project is unchanged.
    pub fn execute(&mut self, mut command: Box<dyn Command>) -> Result<(), CommandError> {
        let mut draft = (*self.project).clone();
        command.apply(&mut draft)?;
        self.project = Arc::new(draft);
        self.redo.clear();
        self.undo.push(command);
        if self.undo.len() > self.history_limit {
            self.undo.remove(0);
        }
        self.revision += 1;
        Ok(())
    }

    /// Returns `false` if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(mut command) = self.undo.pop() else {
            return false;
        };
        command.revert(Arc::make_mut(&mut self.project));
        self.redo.push(command);
        self.revision += 1;
        true
    }

    /// Returns `false` if there was nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(mut command) = self.redo.pop() else {
            return false;
        };
        let project = Arc::make_mut(&mut self.project);
        // Redo replays a command on the exact state it was originally applied to, so it cannot
        // fail unless a command is buggy. Drop the redo stack rather than corrupt the project.
        if command.apply(project).is_err() {
            self.redo.clear();
            return false;
        }
        self.undo.push(command);
        self.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddTrack, RemoveTrack, RenameProject};
    use crate::model::{TrackId, TrackKind};

    #[test]
    fn execute_undo_redo_round_trip() {
        let mut doc = Document::new(Project::new("Test"));
        let original = doc.snapshot();

        doc.execute(Box::new(AddTrack::new(TrackKind::Text, None)))
            .unwrap();
        doc.execute(Box::new(RenameProject::new("Renamed")))
            .unwrap();
        let edited = doc.snapshot();
        assert_eq!(doc.revision(), 2);

        assert!(doc.undo());
        assert!(doc.undo());
        assert!(!doc.undo());
        assert_eq!(*doc.snapshot(), *original);

        assert!(doc.redo());
        assert!(doc.redo());
        assert!(!doc.redo());
        assert_eq!(*doc.snapshot(), *edited);
        assert_eq!(doc.revision(), 6);
    }

    #[test]
    fn failed_command_changes_nothing() {
        let mut doc = Document::new(Project::new("Test"));
        let before = doc.snapshot();
        assert!(
            doc.execute(Box::new(RemoveTrack::new(TrackId::new())))
                .is_err()
        );
        assert_eq!(*doc.snapshot(), *before);
        assert_eq!(doc.revision(), 0);
        assert!(!doc.can_undo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut doc = Document::new(Project::new("Test"));
        doc.execute(Box::new(RenameProject::new("A"))).unwrap();
        doc.undo();
        assert!(doc.can_redo());
        doc.execute(Box::new(RenameProject::new("B"))).unwrap();
        assert!(!doc.can_redo());
    }

    #[test]
    fn snapshots_are_not_affected_by_later_edits() {
        let mut doc = Document::new(Project::new("Test"));
        let snapshot = doc.snapshot();
        doc.execute(Box::new(RenameProject::new("Changed")))
            .unwrap();
        doc.undo();
        doc.redo();
        assert_eq!(snapshot.name, "Test");
    }

    #[test]
    fn history_is_capped() {
        let mut doc = Document::new(Project::new("Test"));
        doc.history_limit = 3;
        for i in 0..5 {
            doc.execute(Box::new(RenameProject::new(format!("N{i}"))))
                .unwrap();
        }
        let mut undone = 0;
        while doc.undo() {
            undone += 1;
        }
        assert_eq!(undone, 3);
        assert_eq!(doc.project().name, "N1");
    }
}
