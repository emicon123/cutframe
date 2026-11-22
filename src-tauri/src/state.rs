//! Application state shared by commands, the media protocol and the preview.

use std::sync::{Arc, Mutex, MutexGuard};

use app_core::command::CommandError;
use app_core::{Document, Project};
use arc_swap::ArcSwap;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::ipc::IpcError;
use crate::ipc::events::ProjectChanged;
use crate::ipc::types::ProjectState;
use crate::preview::Preview;

#[derive(Debug)]
pub struct AppState {
    session: Mutex<Session>,
    /// Latest project snapshot for engine threads. Lock-free reads.
    snapshot: ArcSwap<Project>,
    pub preview: Option<Preview>,
}

impl AppState {
    pub fn new(project: Project, preview: Option<Preview>) -> Self {
        let session = Session::new(project);
        let snapshot = ArcSwap::new(session.document.snapshot());
        if let Some(preview) = &preview {
            preview.set_canvas_aspect(&session.document.project().settings);
        }
        Self {
            session: Mutex::new(session),
            snapshot,
            preview,
        }
    }

    pub fn project_state(&self) -> ProjectState {
        self.lock().state()
    }

    #[allow(dead_code)] // Read by the render graph from stage 1 on.
    pub fn snapshot(&self) -> Arc<Project> {
        self.snapshot.load_full()
    }

    /// Runs `edit` on the document, publishes the new snapshot and notifies the UI.
    ///
    /// `edit` returns `Ok(None)` when nothing changed (e.g. undo with an empty history); then the
    /// revision stays the same and no event is emitted.
    pub fn edit<T>(
        &self,
        app: &AppHandle,
        edit: impl FnOnce(&mut Document) -> Result<Option<T>, CommandError>,
    ) -> Result<Option<T>, IpcError> {
        let (result, state) = {
            let mut session = self.lock();
            let Some(result) = session.edit(edit)? else {
                return Ok(None);
            };
            self.publish(&session.document);
            (result, session.state())
        };
        // Emit outside the lock. Events may arrive out of order; the UI orders them by revision.
        ProjectChanged(state).emit(app)?;
        Ok(Some(result))
    }

    /// Replaces the open document with `project` (new or opened project) and notifies the UI.
    /// The history starts empty; the revision keeps counting up.
    #[allow(dead_code)] // Used by the new/open project commands (target).
    pub fn replace_project(&self, app: &AppHandle, project: Project) -> Result<(), IpcError> {
        let state = {
            let mut session = self.lock();
            session.replace(project);
            self.publish(&session.document);
            session.state()
        };
        ProjectChanged(state).emit(app)?;
        Ok(())
    }

    /// Hands the document's current state to the engine.
    fn publish(&self, document: &Document) {
        self.snapshot.store(document.snapshot());
        if let Some(preview) = &self.preview {
            preview.set_canvas_aspect(&document.project().settings);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Session> {
        // A panic while holding the lock leaves a consistent document: commands mutate a draft
        // and swap it in only on success.
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The open document and the session-wide revision. They share one lock, so every
/// [`ProjectState`] carries the revision of exactly the state it was built from.
#[derive(Debug)]
struct Session {
    document: Document,
    /// Increases on every change and is monotonic for the whole app session (ADR-0003 §6): it
    /// is never reset when the document is replaced, so the UI's "keep the highest revision" rule
    /// keeps working across new/open project.
    revision: u64,
}

impl Session {
    fn new(project: Project) -> Self {
        Self {
            document: Document::new(project),
            revision: 0,
        }
    }

    fn state(&self) -> ProjectState {
        ProjectState::new(&self.document, self.revision)
    }

    /// Runs `edit`; bumps the revision only if it reports a change.
    fn edit<T>(
        &mut self,
        edit: impl FnOnce(&mut Document) -> Result<Option<T>, CommandError>,
    ) -> Result<Option<T>, CommandError> {
        let result = edit(&mut self.document)?;
        if result.is_some() {
            self.revision += 1;
        }
        Ok(result)
    }

    fn replace(&mut self, project: Project) {
        self.document = Document::new(project);
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use app_core::command::{AddTrack, RemoveTrack, RenameProject};
    use app_core::{TrackId, TrackKind};

    use super::*;

    fn rename(session: &mut Session, name: &str) {
        session
            .edit(|doc| doc.execute(Box::new(RenameProject::new(name))).map(Some))
            .expect("rename");
    }

    #[test]
    fn revision_increases_only_when_something_changed() {
        let mut session = Session::new(Project::new("Test"));
        assert_eq!(session.state().revision, 0);

        rename(&mut session, "A");
        assert_eq!(session.state().revision, 1);

        // Nothing to redo: no change, no new revision.
        let redone = session.edit(|doc| Ok(doc.redo().then_some(()))).unwrap();
        assert_eq!(redone, None);
        assert_eq!(session.state().revision, 1);

        // A rejected edit leaves the revision alone.
        let failed = session.edit(|doc| {
            doc.execute(Box::new(RemoveTrack::new(TrackId::new())))
                .map(Some)
        });
        assert!(failed.is_err());
        assert_eq!(session.state().revision, 1);

        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert_eq!(session.state().revision, 2);
    }

    #[test]
    fn every_gesture_step_bumps_the_revision() {
        let mut session = Session::new(Project::new("Test"));
        for (i, name) in ["A", "AB", "ABC"].into_iter().enumerate() {
            session
                .edit(|doc| {
                    doc.gesture_update(Box::new(RenameProject::new(name)))
                        .map(Some)
                })
                .unwrap();
            assert_eq!(session.state().revision, i as u64 + 1);
            assert_eq!(
                session.state().project.name,
                name,
                "live state is published"
            );
        }

        let cancelled = session.edit(|doc| Ok(doc.gesture_cancel().then_some(())));
        assert_eq!(cancelled.unwrap(), Some(()));
        assert_eq!(session.state().revision, 4);
        assert_eq!(session.state().project.name, "Test");

        // Nothing left to cancel: no change, no new revision.
        let cancelled = session.edit(|doc| Ok(doc.gesture_cancel().then_some(())));
        assert_eq!(cancelled.unwrap(), None);
        assert_eq!(session.state().revision, 4);
    }

    #[test]
    fn replacing_the_document_never_lowers_the_revision() {
        let mut session = Session::new(Project::new("First"));
        let mut emitted = vec![session.state().revision];
        for name in ["A", "B", "C"] {
            rename(&mut session, name);
            emitted.push(session.state().revision);
        }
        session
            .edit(|doc| Ok(doc.undo().then_some(())))
            .unwrap()
            .expect("undo");
        emitted.push(session.state().revision);
        let highest = *emitted.iter().max().unwrap();

        session.replace(Project::new("Second"));
        let state = session.state();
        assert!(state.revision > highest, "{} <= {highest}", state.revision);
        assert_eq!(state.project.name, "Second");
        assert!(!state.history.can_undo && !state.history.can_redo);

        // Edits on the new document keep counting from there.
        session
            .edit(|doc| {
                doc.execute(Box::new(AddTrack::new(TrackKind::Video, None)))
                    .map(Some)
            })
            .unwrap();
        assert_eq!(session.state().revision, state.revision + 1);
    }
}
