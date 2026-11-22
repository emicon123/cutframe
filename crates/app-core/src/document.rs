//! An open project: the current state, its undo/redo history and the open gesture, if any.
//!
//! The revision counter sent to the UI is not kept here: it must stay monotonic across documents
//! for the whole app session, so the shell owns it (ADR-0003 §6).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandError, HistoryLabel};
use crate::model::Project;

/// Maximum number of undo steps kept in memory. A batch or a gesture counts as one.
pub const DEFAULT_HISTORY_LIMIT: usize = 500;

/// Which step of a continuous edit (drag, trim, slider) a command is (ADR-0006 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum EditPhase {
    /// Show this value live; opens a gesture if none is open. Records nothing.
    Update,
    /// Final value: records one history entry. Without an open gesture this is a plain edit
    /// (keyboard nudge, typed value).
    Commit,
}

/// The editable document. Owned by one thread; readers get cheap [`Arc`] snapshots.
///
/// Every edit applies its command to a clone of the current state and swaps the clone in only on
/// success, so a failed command leaves the project untouched. Undo and redo mutate in place
/// (copy-on-write via [`Arc::make_mut`]: they clone only while a snapshot is held elsewhere).
#[derive(Debug)]
pub struct Document {
    project: Arc<Project>,
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
    history_limit: usize,
    gesture: Option<Gesture>,
}

/// An open continuous edit ("replace from base", ADR-0006 §2).
#[derive(Debug)]
struct Gesture {
    /// The state before the gesture. Every update and the commit apply to a clone of it.
    base: Arc<Project>,
    /// Kind of the gesture's commands; updates and the commit of another kind are rejected.
    label: HistoryLabel,
}

impl Document {
    pub fn new(project: Project) -> Self {
        Self {
            project: Arc::new(project),
            undo: Vec::new(),
            redo: Vec::new(),
            history_limit: DEFAULT_HISTORY_LIMIT,
            gesture: None,
        }
    }

    /// The current state, including an open gesture's live value. Cheap to clone and safe to send
    /// to other threads.
    pub fn snapshot(&self) -> Arc<Project> {
        Arc::clone(&self.project)
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The state to save: an open gesture's base, so a half-finished gesture is never written
    /// (ADR-0006 §3). Otherwise the same as [`Document::snapshot`]. Commits nothing.
    pub fn committed(&self) -> Arc<Project> {
        Arc::clone(self.gesture.as_ref().map_or(&self.project, |g| &g.base))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Key of the entry the next `undo` reverts.
    pub fn undo_label(&self) -> Option<HistoryLabel> {
        self.undo.last().map(|c| c.label())
    }

    /// Key of the entry the next `redo` re-applies.
    pub fn redo_label(&self) -> Option<HistoryLabel> {
        self.redo.last().map(|c| c.label())
    }

    /// Key of the open gesture, if any.
    pub fn gesture_label(&self) -> Option<HistoryLabel> {
        self.gesture.as_ref().map(|g| g.label)
    }

    /// Applies `command` and records it. On error, the project is unchanged.
    ///
    /// Fails with [`CommandError::GestureInProgress`] while a gesture is open.
    pub fn execute(&mut self, mut command: Box<dyn Command>) -> Result<(), CommandError> {
        if self.gesture.is_some() {
            return Err(CommandError::GestureInProgress);
        }
        self.project = Arc::new(applied(&self.project, command.as_mut())?);
        self.record(command);
        Ok(())
    }

    /// Runs `command` as the given step of a gesture: [`Document::gesture_update`] or
    /// [`Document::gesture_commit`].
    pub fn edit(
        &mut self,
        phase: EditPhase,
        command: Box<dyn Command>,
    ) -> Result<(), CommandError> {
        match phase {
            EditPhase::Update => self.gesture_update(command),
            EditPhase::Commit => self.gesture_commit(command),
        }
    }

    /// Shows `command` live without recording it. Opens a gesture (base = current state) if none is
    /// open, otherwise replaces the previous update: `command` is applied to a clone of the base,
    /// so it must carry absolute values (new start, new value), never deltas.
    ///
    /// On error nothing changes: the previous update stays visible and the gesture stays open (or
    /// is not opened). Fails with [`CommandError::GestureMismatch`] if `command` is a different
    /// kind than the open gesture.
    pub fn gesture_update(&mut self, mut command: Box<dyn Command>) -> Result<(), CommandError> {
        let base = self.gesture_base(command.as_ref())?;
        let draft = applied(&base, command.as_mut())?;
        if self.gesture.is_none() {
            self.gesture = Some(Gesture {
                base,
                label: command.label(),
            });
        }
        self.project = Arc::new(draft);
        Ok(())
    }

    /// Applies `command` to the open gesture's base and records it as one history entry, closing
    /// the gesture. Without an open gesture this is [`Document::execute`].
    ///
    /// On error nothing changes and the gesture stays open; the caller commits another value or
    /// calls [`Document::gesture_cancel`].
    pub fn gesture_commit(&mut self, mut command: Box<dyn Command>) -> Result<(), CommandError> {
        if self.gesture.is_none() {
            return self.execute(command);
        }
        let base = self.gesture_base(command.as_ref())?;
        self.project = Arc::new(applied(&base, command.as_mut())?);
        self.gesture = None;
        self.record(command);
        Ok(())
    }

    /// Restores the open gesture's base and records nothing. Returns `false` if no gesture was
    /// open.
    pub fn gesture_cancel(&mut self) -> bool {
        let Some(gesture) = self.gesture.take() else {
            return false;
        };
        self.project = gesture.base;
        true
    }

    /// Returns `false` if there was nothing to undo. While a gesture is open, cancels it and does
    /// nothing else (ADR-0006 §3).
    pub fn undo(&mut self) -> bool {
        if self.gesture_cancel() {
            return true;
        }
        let Some(mut command) = self.undo.pop() else {
            return false;
        };
        command.revert(Arc::make_mut(&mut self.project));
        self.redo.push(command);
        true
    }

    /// Returns `false` if there was nothing to redo. While a gesture is open, cancels it and does
    /// nothing else (ADR-0006 §3).
    pub fn redo(&mut self) -> bool {
        if self.gesture_cancel() {
            return true;
        }
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
        true
    }

    /// The state a gesture step applies to: the open gesture's base, or the current state when
    /// `command` opens a new gesture.
    fn gesture_base(&self, command: &dyn Command) -> Result<Arc<Project>, CommandError> {
        match &self.gesture {
            None => Ok(self.snapshot()),
            Some(gesture) if gesture.label == command.label() => Ok(Arc::clone(&gesture.base)),
            Some(gesture) => Err(CommandError::GestureMismatch {
                open: gesture.label,
                got: command.label(),
            }),
        }
    }

    /// Pushes an applied command as one history entry.
    fn record(&mut self, command: Box<dyn Command>) {
        self.redo.clear();
        self.undo.push(command);
        if self.undo.len() > self.history_limit {
            self.undo.remove(0);
        }
    }
}

/// `command` applied to a clone of `project`.
fn applied(project: &Project, command: &mut dyn Command) -> Result<Project, CommandError> {
    let mut draft = project.clone();
    command.apply(&mut draft)?;
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddTrack, Batch, RemoveTrack, RenameProject, RenameTrack};
    use crate::model::{TrackId, TrackKind};
    use crate::serialization;

    fn rename(name: &str) -> Box<dyn Command> {
        Box::new(RenameProject::new(name))
    }

    #[test]
    fn execute_undo_redo_round_trip() {
        let mut doc = Document::new(Project::new("Test"));
        let original = doc.snapshot();

        doc.execute(Box::new(AddTrack::new(TrackKind::Text, None)))
            .unwrap();
        doc.execute(rename("Renamed")).unwrap();
        let edited = doc.snapshot();

        assert!(doc.undo());
        assert!(doc.undo());
        assert!(!doc.undo());
        assert_eq!(*doc.snapshot(), *original);

        assert!(doc.redo());
        assert!(doc.redo());
        assert!(!doc.redo());
        assert_eq!(*doc.snapshot(), *edited);
    }

    #[test]
    fn history_labels_follow_the_stacks() {
        let mut doc = Document::new(Project::new("Test"));
        assert_eq!((doc.undo_label(), doc.redo_label()), (None, None));

        doc.execute(Box::new(AddTrack::new(TrackKind::Text, None)))
            .unwrap();
        doc.execute(rename("Renamed")).unwrap();
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RenameProject));
        assert_eq!(doc.redo_label(), None);

        doc.undo();
        assert_eq!(doc.undo_label(), Some(HistoryLabel::AddTrack));
        assert_eq!(doc.redo_label(), Some(HistoryLabel::RenameProject));
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
        assert!(!doc.can_undo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut doc = Document::new(Project::new("Test"));
        doc.execute(rename("A")).unwrap();
        doc.undo();
        assert!(doc.can_redo());
        doc.execute(rename("B")).unwrap();
        assert!(!doc.can_redo());
    }

    #[test]
    fn snapshots_are_not_affected_by_later_edits() {
        let mut doc = Document::new(Project::new("Test"));
        let snapshot = doc.snapshot();
        doc.execute(rename("Changed")).unwrap();
        doc.undo();
        doc.redo();
        assert_eq!(snapshot.name, "Test");
    }

    #[test]
    fn history_is_capped() {
        let mut doc = Document::new(Project::new("Test"));
        doc.history_limit = 3;
        for i in 0..5 {
            doc.execute(rename(&format!("N{i}"))).unwrap();
        }
        let mut undone = 0;
        while doc.undo() {
            undone += 1;
        }
        assert_eq!(undone, 3);
        assert_eq!(doc.project().name, "N1");
    }

    #[test]
    fn failed_batch_changes_nothing_and_records_nothing() {
        let mut doc = Document::new(Project::new("Test"));
        let before = doc.snapshot();
        let first = before.tracks[0].id;
        let batch = Batch::new(
            HistoryLabel::RemoveTrack,
            vec![
                Box::new(RemoveTrack::new(first)),
                Box::new(RemoveTrack::new(TrackId::new())),
            ],
        );
        assert!(doc.execute(Box::new(batch)).is_err());
        assert_eq!(*doc.snapshot(), *before);
        assert!(!doc.can_undo());
    }

    #[test]
    fn batch_is_one_history_entry() {
        let mut doc = Document::new(Project::new("Test"));
        let before = doc.snapshot();
        let ids: Vec<TrackId> = before.tracks.iter().map(|t| t.id).collect();
        let batch = Batch::new(
            HistoryLabel::RemoveTrack,
            ids.iter()
                .map(|&id| Box::new(RemoveTrack::new(id)) as Box<dyn Command>)
                .collect(),
        );
        doc.execute(Box::new(batch)).unwrap();
        assert!(doc.project().tracks.is_empty());
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RemoveTrack));

        assert!(doc.undo());
        assert_eq!(*doc.snapshot(), *before);
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert!(doc.project().tracks.is_empty());
    }

    #[test]
    fn gesture_updates_and_commit_make_one_undo_entry() {
        let mut doc = Document::new(Project::new("Test"));
        let base = doc.snapshot();

        for name in ["A", "AB", "ABC"] {
            doc.gesture_update(rename(name)).unwrap();
            assert_eq!(doc.project().name, name, "updates are shown live");
            assert!(!doc.can_undo(), "updates record nothing");
        }
        assert_eq!(doc.gesture_label(), Some(HistoryLabel::RenameProject));

        doc.gesture_commit(rename("Final")).unwrap();
        assert_eq!(doc.project().name, "Final");
        assert_eq!(doc.gesture_label(), None);
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RenameProject));

        assert!(doc.undo());
        assert_eq!(*doc.snapshot(), *base, "one undo reverts the whole gesture");
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(doc.project().name, "Final");
    }

    #[test]
    fn commit_without_a_gesture_is_a_plain_execute() {
        let mut doc = Document::new(Project::new("Test"));
        doc.edit(EditPhase::Commit, rename("Typed")).unwrap();
        assert_eq!(doc.project().name, "Typed");
        assert_eq!(doc.gesture_label(), None);
        assert!(doc.undo());
        assert_eq!(doc.project().name, "Test");
    }

    #[test]
    fn edit_phase_dispatches_to_the_gesture_api() {
        let mut doc = Document::new(Project::new("Test"));
        doc.edit(EditPhase::Update, rename("Live")).unwrap();
        assert_eq!(doc.gesture_label(), Some(HistoryLabel::RenameProject));
        doc.edit(EditPhase::Commit, rename("Done")).unwrap();
        assert_eq!(doc.gesture_label(), None);
        assert_eq!(doc.project().name, "Done");
    }

    #[test]
    fn cancel_restores_the_base_exactly() {
        let mut doc = Document::new(Project::new("Test"));
        doc.execute(rename("Before")).unwrap();
        doc.undo();
        let base = doc.snapshot();

        doc.gesture_update(rename("A")).unwrap();
        doc.gesture_update(rename("B")).unwrap();
        assert!(doc.gesture_cancel());
        assert_eq!(*doc.snapshot(), *base);
        assert!(
            Arc::ptr_eq(&doc.snapshot(), &base),
            "the base itself is restored"
        );
        assert_eq!(doc.gesture_label(), None);
        assert!(!doc.can_undo());
        assert!(doc.can_redo(), "a cancelled gesture keeps the redo stack");
        assert!(!doc.gesture_cancel(), "nothing left to cancel");
    }

    #[test]
    fn execute_is_rejected_while_a_gesture_is_open() {
        let mut doc = Document::new(Project::new("Test"));
        doc.gesture_update(rename("Live")).unwrap();
        let live = doc.snapshot();
        assert_eq!(
            doc.execute(Box::new(AddTrack::new(TrackKind::Text, None))),
            Err(CommandError::GestureInProgress)
        );
        assert_eq!(*doc.snapshot(), *live);
        assert!(!doc.can_undo());
    }

    #[test]
    fn undo_and_redo_during_a_gesture_only_cancel_it() {
        let mut doc = Document::new(Project::new("Test"));
        doc.execute(rename("One")).unwrap();
        doc.execute(rename("Two")).unwrap();
        doc.undo();
        let base = doc.snapshot();

        doc.gesture_update(rename("Live")).unwrap();
        assert!(doc.undo());
        assert_eq!(*doc.snapshot(), *base, "undo cancelled the gesture only");
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RenameProject));
        assert!(doc.can_redo());

        doc.gesture_update(rename("Live")).unwrap();
        assert!(doc.redo());
        assert_eq!(*doc.snapshot(), *base, "redo cancelled the gesture only");
        assert!(doc.can_redo());

        // The history is intact: the next undo and redo work as before the gesture.
        assert!(doc.redo());
        assert_eq!(doc.project().name, "Two");
        assert!(doc.undo());
        assert!(doc.undo());
        assert_eq!(doc.project().name, "Test");
    }

    #[test]
    fn a_different_command_kind_is_rejected_during_a_gesture() {
        let mut doc = Document::new(Project::new("Test"));
        let track = doc.project().tracks[0].id;
        doc.gesture_update(rename("Live")).unwrap();
        let live = doc.snapshot();
        let expected = Err(CommandError::GestureMismatch {
            open: HistoryLabel::RenameProject,
            got: HistoryLabel::RenameTrack,
        });

        assert_eq!(
            doc.gesture_update(Box::new(RenameTrack::new(track, "X"))),
            expected
        );
        assert_eq!(
            doc.gesture_commit(Box::new(RenameTrack::new(track, "X"))),
            expected
        );
        assert_eq!(*doc.snapshot(), *live);
        assert_eq!(doc.gesture_label(), Some(HistoryLabel::RenameProject));
    }

    #[test]
    fn failed_gesture_steps_change_nothing() {
        let mut doc = Document::new(Project::new("Test"));
        let base = doc.snapshot();

        // A failing first update does not open a gesture.
        assert_eq!(
            doc.gesture_update(rename(" ")),
            Err(CommandError::EmptyName)
        );
        assert_eq!(doc.gesture_label(), None);
        assert_eq!(*doc.snapshot(), *base);

        // A failing later update keeps the previous live value; a failing commit keeps the gesture.
        doc.gesture_update(rename("Live")).unwrap();
        assert_eq!(doc.gesture_update(rename("")), Err(CommandError::EmptyName));
        assert_eq!(doc.project().name, "Live");
        assert_eq!(doc.gesture_commit(rename("")), Err(CommandError::EmptyName));
        assert_eq!(doc.project().name, "Live");
        assert_eq!(doc.gesture_label(), Some(HistoryLabel::RenameProject));
        assert!(!doc.can_undo());

        assert!(doc.gesture_cancel());
        assert_eq!(*doc.snapshot(), *base);
    }

    #[test]
    fn saving_during_a_gesture_writes_the_base() {
        let mut doc = Document::new(Project::new("Saved"));
        let base = doc.snapshot();
        doc.gesture_update(rename("Half-finished")).unwrap();

        let saved = serialization::from_json(&serialization::to_json(&doc.committed())).unwrap();
        assert_eq!(saved, *base);
        assert_eq!(
            doc.project().name,
            "Half-finished",
            "saving commits nothing"
        );
        assert_eq!(doc.gesture_label(), Some(HistoryLabel::RenameProject));
        assert!(!doc.can_undo());

        doc.gesture_commit(rename("Committed")).unwrap();
        assert_eq!(doc.committed().name, "Committed");
    }

    #[test]
    fn history_limit_counts_a_gesture_as_one_entry() {
        let mut doc = Document::new(Project::new("Test"));
        doc.history_limit = 3;
        for i in 0..5 {
            for step in 0..10 {
                doc.gesture_update(rename(&format!("G{i}.{step}"))).unwrap();
            }
            doc.gesture_commit(rename(&format!("G{i}"))).unwrap();
        }
        let mut undone = 0;
        while doc.undo() {
            undone += 1;
        }
        assert_eq!(undone, 3);
        assert_eq!(doc.project().name, "G1");
    }

    mod properties {
        use proptest::prelude::*;

        use super::*;

        #[derive(Debug, Clone)]
        enum Op {
            Rename(String),
            AddTrack,
            RemoveTrack(usize),
            Update { track: bool, name: String },
            Commit { track: bool, name: String },
            Cancel,
            Undo,
            Redo,
        }

        fn name() -> impl Strategy<Value = String> {
            // Includes blank names, which commands reject.
            "[ab ]{0,2}"
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                name().prop_map(Op::Rename),
                Just(Op::AddTrack),
                (0usize..4).prop_map(Op::RemoveTrack),
                (any::<bool>(), name()).prop_map(|(track, name)| Op::Update { track, name }),
                (any::<bool>(), name()).prop_map(|(track, name)| Op::Commit { track, name }),
                Just(Op::Cancel),
                Just(Op::Undo),
                Just(Op::Redo),
            ]
        }

        /// A gesture-capable command with an absolute value: rename the project or the first track.
        fn gesture_command(project: &Project, track: bool, name: String) -> Box<dyn Command> {
            match project.tracks.first() {
                Some(t) if track => Box::new(RenameTrack::new(t.id, name)),
                _ => Box::new(RenameProject::new(name)),
            }
        }

        /// Records the document's state as a new history entry after the cursor.
        fn record(doc: &Document, states: &mut Vec<Arc<Project>>, cursor: &mut usize) {
            states.truncate(*cursor + 1);
            states.push(doc.snapshot());
            *cursor = states.len() - 1;
        }

        proptest! {
            /// Checks the document against a reference model: a list of committed states with a
            /// cursor, plus the open gesture's base and kind.
            #[test]
            fn history_matches_a_reference_model(ops in prop::collection::vec(op(), 0..60)) {
                let mut doc = Document::new(Project::new("Start"));
                let mut states = vec![doc.snapshot()];
                let mut cursor = 0;
                let mut gesture: Option<HistoryLabel> = None;

                for op in ops {
                    match op {
                        Op::Rename(name) => {
                            let result = doc.execute(Box::new(RenameProject::new(name)));
                            if gesture.is_some() {
                                prop_assert_eq!(result, Err(CommandError::GestureInProgress));
                            } else if result.is_ok() {
                                record(&doc, &mut states, &mut cursor);
                            }
                        }
                        Op::AddTrack => {
                            let result =
                                doc.execute(Box::new(AddTrack::new(TrackKind::Video, None)));
                            prop_assert_eq!(result.is_ok(), gesture.is_none());
                            if result.is_ok() {
                                record(&doc, &mut states, &mut cursor);
                            }
                        }
                        Op::RemoveTrack(index) => {
                            let id = doc.project().tracks.get(index).map_or_else(TrackId::new, |t| t.id);
                            if doc.execute(Box::new(RemoveTrack::new(id))).is_ok() {
                                prop_assert!(gesture.is_none());
                                record(&doc, &mut states, &mut cursor);
                            }
                        }
                        Op::Update { track, name } => {
                            let command = gesture_command(&doc.committed(), track, name);
                            let label = command.label();
                            if doc.gesture_update(command).is_ok() {
                                prop_assert!(gesture.is_none_or(|open| open == label));
                                gesture = Some(label);
                            }
                        }
                        Op::Commit { track, name } => {
                            let command = gesture_command(&doc.committed(), track, name);
                            let label = command.label();
                            if doc.gesture_commit(command).is_ok() {
                                prop_assert!(gesture.is_none_or(|open| open == label));
                                gesture = None;
                                record(&doc, &mut states, &mut cursor);
                            }
                        }
                        Op::Cancel => {
                            prop_assert_eq!(doc.gesture_cancel(), gesture.take().is_some());
                        }
                        Op::Undo => {
                            let undone = doc.undo();
                            if gesture.take().is_some() {
                                prop_assert!(undone);
                            } else {
                                prop_assert_eq!(undone, cursor > 0);
                                if undone {
                                    cursor -= 1;
                                }
                            }
                        }
                        Op::Redo => {
                            let redone = doc.redo();
                            if gesture.take().is_some() {
                                prop_assert!(redone);
                            } else {
                                prop_assert_eq!(redone, cursor + 1 < states.len());
                                if redone {
                                    cursor += 1;
                                }
                            }
                        }
                    }

                    prop_assert_eq!(doc.gesture_label(), gesture);
                    prop_assert_eq!(&*doc.committed(), &*states[cursor]);
                    if gesture.is_none() {
                        prop_assert_eq!(&*doc.snapshot(), &*states[cursor]);
                    }
                    prop_assert_eq!(doc.can_undo(), cursor > 0);
                    prop_assert_eq!(doc.can_redo(), cursor + 1 < states.len());
                }

                // Undoing everything returns to the start.
                while doc.undo() {}
                prop_assert_eq!(&*doc.snapshot(), &*states[0]);
            }
        }
    }
}
