//! Application state shared by commands, the media protocol and the preview.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use app_core::command::CommandError;
use app_core::serialization::CURRENT_SCHEMA_VERSION;
use app_core::{Document, HistoryPosition, Project};
use arc_swap::ArcSwap;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::autosave::{AutosaveHandle, Msg};
use crate::ipc::IpcError;
use crate::ipc::events::ProjectChanged;
use crate::ipc::types::ProjectState;
use crate::jobs::JobManager;
use crate::preview::Preview;
use crate::recovery::{RecoveryStore, Restored};

#[derive(Debug)]
pub struct AppState {
    session: Mutex<Session>,
    /// Latest project snapshot for engine threads. Lock-free reads.
    snapshot: ArcSwap<Project>,
    /// Serializes saves, so two saves never write the same file at once or mark clean out of order.
    save_lock: tauri::async_runtime::Mutex<()>,
    pub preview: Option<Preview>,
    /// This process's recovery directory and session id.
    recovery: RecoveryStore,
    /// Set in `setup` once the autosave thread runs; tests run without it.
    autosave: OnceLock<AutosaveHandle>,
    /// Background work (import now; previews and export later).
    pub jobs: JobManager,
}

/// What a save writes, taken under the lock and written without it.
#[derive(Debug, Clone)]
pub struct SaveSnapshot {
    /// The committed state: an open gesture's base, never its live value (ADR-0006 §3).
    pub project: Arc<Project>,
    /// History position of `project`; the document is clean again only while it stays there.
    position: HistoryPosition,
    /// Which document the snapshot came from; a save that finishes after new/open marks nothing.
    generation: u64,
    pub file_path: Option<PathBuf>,
    /// Schema version the open file was loaded from.
    pub source_version: u32,
    /// Unsaved changes at the time of the snapshot.
    pub dirty: bool,
}

impl SaveSnapshot {
    /// The schema version to back up before writing `path`: only when overwriting the file that
    /// was loaded from an older version (ADR-0002 §4).
    pub fn backup_of_version(&self, path: &Path) -> Option<u32> {
        (self.source_version < CURRENT_SCHEMA_VERSION && self.file_path.as_deref() == Some(path))
            .then_some(self.source_version)
    }

    /// Identifies the saved state: equal keys mean equal content (generation, history position).
    pub fn key(&self) -> (u64, HistoryPosition) {
        (self.generation, self.position)
    }
}

impl AppState {
    pub fn new(
        project: Project,
        preview: Option<Preview>,
        recovery: RecoveryStore,
        jobs: JobManager,
    ) -> Self {
        let session = Session::new(project);
        let snapshot = ArcSwap::new(session.document.snapshot());
        if let Some(preview) = &preview {
            preview.set_canvas_aspect(&session.document.project().settings);
        }
        Self {
            session: Mutex::new(session),
            snapshot,
            save_lock: tauri::async_runtime::Mutex::new(()),
            preview,
            recovery,
            autosave: OnceLock::new(),
            jobs,
        }
    }

    pub fn recovery(&self) -> &RecoveryStore {
        &self.recovery
    }

    /// Connects the autosave thread. Only the first call has an effect.
    pub fn set_autosave(&self, handle: AutosaveHandle) {
        let _ = self.autosave.set(handle);
    }

    /// Clean exit: lets the autosave thread delete this session's recovery files (waits ≤ 2 s).
    pub fn shutdown_autosave(&self) {
        if let Some(autosave) = self.autosave.get() {
            autosave.shutdown();
        }
    }

    fn notify_autosave(&self, msg: Msg) {
        if let Some(autosave) = self.autosave.get() {
            autosave.send(msg);
        }
    }

    /// Unsaved changes in the open project.
    pub fn is_dirty(&self) -> bool {
        self.lock().dirty()
    }

    /// Hold this while saving (see [`AppState::save_snapshot`] and [`AppState::mark_saved`]).
    pub fn save_lock(&self) -> &tauri::async_runtime::Mutex<()> {
        &self.save_lock
    }

    /// The state to save. Cheap: clones an `Arc`.
    pub fn save_snapshot(&self) -> SaveSnapshot {
        self.lock().save_snapshot()
    }

    /// Records that `saved` was written to `path`, and notifies the UI (`filePath` and `dirty`
    /// change). Edits made during the write keep the project dirty.
    pub fn mark_saved(
        &self,
        app: &AppHandle,
        saved: &SaveSnapshot,
        path: PathBuf,
    ) -> Result<(), IpcError> {
        let state = {
            let mut session = self.lock();
            if !session.mark_saved(saved, path) {
                return Ok(());
            }
            session.state()
        };
        // The user's file has it now. If edits came in during the write, the next tick rewrites.
        self.notify_autosave(Msg::Discard);
        ProjectChanged(state).emit(app)?;
        Ok(())
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
    /// The history starts empty, the project is clean, and the revision keeps counting up.
    pub fn replace_project(
        &self,
        app: &AppHandle,
        project: Project,
        file_path: Option<PathBuf>,
        source_version: u32,
    ) -> Result<(), IpcError> {
        let state = {
            let mut session = self.lock();
            session.replace(project, file_path, source_version);
            self.publish(&session.document);
            session.state()
        };
        self.notify_autosave(Msg::Discard);
        ProjectChanged(state).emit(app)?;
        Ok(())
    }

    /// Replaces the open document with the recovery entry `old` and notifies the UI. Like
    /// [`AppState::replace_project`], but the project stays dirty until it is saved, and the
    /// autosave thread takes the entry over (writes it as this session's, deletes `old`).
    pub fn replace_restored(
        &self,
        app: &AppHandle,
        restored: Restored,
        old: uuid::Uuid,
    ) -> Result<(), IpcError> {
        let state = {
            let mut session = self.lock();
            session.replace_restored(
                restored.project,
                restored.file_path,
                restored.source_version,
            );
            self.publish(&session.document);
            session.state()
        };
        self.notify_autosave(Msg::Adopt { old });
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

    #[cfg(test)]
    pub fn test_edit(&self, edit: impl FnOnce(&mut Document) -> Result<Option<()>, CommandError>) {
        let mut session = self.lock();
        session.edit(edit).expect("edit");
        self.publish(&session.document);
    }

    #[cfg(test)]
    pub fn test_replace(&self, project: Project, file_path: Option<PathBuf>, source_version: u32) {
        self.lock().replace(project, file_path, source_version);
    }

    #[cfg(test)]
    pub fn test_replace_restored(
        &self,
        project: Project,
        file_path: Option<PathBuf>,
        source_version: u32,
    ) {
        self.lock()
            .replace_restored(project, file_path, source_version);
    }

    #[cfg(test)]
    pub fn test_mark_saved(&self, saved: &SaveSnapshot, path: PathBuf) {
        assert!(self.lock().mark_saved(saved, path));
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
    /// The file the project was opened from or last saved to.
    file_path: Option<PathBuf>,
    /// History position at the last save or open: the project is clean while it stays there.
    /// `None` for a restored autosave until its first save: no undo or redo makes it clean.
    saved_at: Option<HistoryPosition>,
    /// `schemaVersion` the open file was loaded from; current after the first save.
    source_version: u32,
    /// Increases when the document is replaced. History positions of different documents are
    /// not comparable.
    generation: u64,
}

impl Session {
    fn new(project: Project) -> Self {
        let document = Document::new(project);
        Self {
            saved_at: Some(document.position()),
            document,
            revision: 0,
            file_path: None,
            source_version: CURRENT_SCHEMA_VERSION,
            generation: 0,
        }
    }

    fn state(&self) -> ProjectState {
        ProjectState::new(
            &self.document,
            self.revision,
            self.file_path.as_deref(),
            self.dirty(),
        )
    }

    fn dirty(&self) -> bool {
        self.saved_at != Some(self.document.position()) || self.document.has_gesture()
    }

    fn save_snapshot(&self) -> SaveSnapshot {
        // With an open gesture, `committed()` is its base and `position()` still the base's
        // position (gesture updates record nothing), so the pair always matches.
        SaveSnapshot {
            project: self.document.committed(),
            position: self.document.position(),
            generation: self.generation,
            file_path: self.file_path.clone(),
            source_version: self.source_version,
            dirty: self.dirty(),
        }
    }

    /// Returns `false` (and changes nothing) if the document was replaced since `saved` was taken.
    fn mark_saved(&mut self, saved: &SaveSnapshot, path: PathBuf) -> bool {
        if saved.generation != self.generation {
            return false;
        }
        self.saved_at = Some(saved.position);
        self.file_path = Some(path);
        self.source_version = CURRENT_SCHEMA_VERSION;
        self.revision += 1;
        true
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

    fn replace(&mut self, project: Project, file_path: Option<PathBuf>, source_version: u32) {
        self.document = Document::new(project);
        self.saved_at = Some(self.document.position());
        self.file_path = file_path;
        self.source_version = source_version;
        self.generation += 1;
        self.revision += 1;
    }

    /// [`Session::replace`] for a restored autosave: dirty until the first explicit save.
    fn replace_restored(
        &mut self,
        project: Project,
        file_path: Option<PathBuf>,
        source_version: u32,
    ) {
        self.replace(project, file_path, source_version);
        self.saved_at = None;
    }
}

#[cfg(test)]
mod tests {
    use app_core::command::{AddTrack, HistoryLabel, RemoveTrack, RenameProject};
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

        assert!(session.state().dirty);

        session.replace(
            Project::new("Second"),
            Some(PathBuf::from("/projects/second.cutframe")),
            CURRENT_SCHEMA_VERSION,
        );
        let state = session.state();
        assert!(state.revision > highest, "{} <= {highest}", state.revision);
        assert_eq!(state.project.name, "Second");
        assert!(!state.history.can_undo && !state.history.can_redo);
        assert!(!state.dirty, "a just-opened project is clean");
        assert!(state.file_path.is_some());

        // Edits on the new document keep counting from there.
        session
            .edit(|doc| {
                doc.execute(Box::new(AddTrack::new(TrackKind::Video, None)))
                    .map(Some)
            })
            .unwrap();
        assert_eq!(session.state().revision, state.revision + 1);
    }

    fn jump(session: &mut Session, index: usize) -> Option<()> {
        session
            .edit(|doc| Ok(doc.jump_to(index)?.then_some(())))
            .expect("jump")
    }

    #[test]
    fn a_history_jump_is_one_revision_and_keeps_dirty_tracking() {
        let mut session = Session::new(Project::new("Test"));
        for name in ["A", "B", "C"] {
            rename(&mut session, name);
        }
        save(&mut session, "/p/a.cutframe");
        let before = session.state().revision;

        // Three undo steps, one change.
        assert_eq!(jump(&mut session, 0), Some(()));
        let state = session.state();
        assert_eq!(state.revision, before + 1);
        assert_eq!(state.project.name, "Test");
        assert_eq!(state.history.current, 0);
        assert_eq!(
            state
                .history
                .entries
                .iter()
                .map(|e| e.label)
                .collect::<Vec<_>>(),
            [HistoryLabel::RenameProject; 3]
        );
        assert!(state.dirty);

        // Already there: no change, no revision.
        assert_eq!(jump(&mut session, 0), None);
        assert_eq!(session.state().revision, before + 1);

        // Back to the saved point is clean again.
        assert_eq!(jump(&mut session, 3), Some(()));
        let state = session.state();
        assert_eq!(state.revision, before + 2);
        assert_eq!((state.history.current, state.dirty), (3, false));

        // Out of range: rejected, nothing changes.
        assert_eq!(
            session.edit(|doc| Ok(doc.jump_to(4)?.then_some(()))),
            Err(CommandError::HistoryOutOfRange { index: 4, len: 3 })
        );
        assert_eq!(session.state().revision, before + 2);
    }

    fn save(session: &mut Session, path: &str) -> SaveSnapshot {
        let saved = session.save_snapshot();
        assert!(session.mark_saved(&saved, PathBuf::from(path)));
        saved
    }

    #[test]
    fn a_new_session_is_clean_and_has_no_file() {
        let session = Session::new(Project::new("Test"));
        let state = session.state();
        assert!(!state.dirty);
        assert_eq!(state.file_path, None);
    }

    #[test]
    fn saving_marks_clean_and_undo_back_to_the_save_is_clean() {
        let mut session = Session::new(Project::new("Test"));
        rename(&mut session, "A");
        assert!(session.state().dirty);

        let before = session.state().revision;
        save(&mut session, "/p/a.cutframe");
        let state = session.state();
        assert!(!state.dirty);
        assert!(state.revision > before, "save emits a new state");
        assert_eq!(state.file_path.as_deref(), Some("/p/a.cutframe"));

        rename(&mut session, "B");
        assert!(session.state().dirty);
        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert!(!session.state().dirty, "back at the saved point");
        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert!(session.state().dirty, "before the saved point");
    }

    #[test]
    fn an_edit_during_the_write_keeps_the_project_dirty() {
        let mut session = Session::new(Project::new("Test"));
        rename(&mut session, "Saved");
        let saved = session.save_snapshot();
        rename(&mut session, "Edited while writing");
        assert!(session.mark_saved(&saved, PathBuf::from("/p/a.cutframe")));
        assert!(session.state().dirty);
        assert_eq!(saved.project.name, "Saved");

        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert!(!session.state().dirty, "the written state is clean");
    }

    #[test]
    fn saving_during_a_gesture_writes_the_base_and_stays_dirty() {
        let mut session = Session::new(Project::new("Base"));
        session
            .edit(|doc| {
                doc.gesture_update(Box::new(RenameProject::new("Live")))
                    .map(Some)
            })
            .unwrap();
        let saved = save(&mut session, "/p/a.cutframe");
        assert_eq!(saved.project.name, "Base");
        assert!(session.state().dirty, "the open gesture is unsaved");

        session
            .edit(|doc| Ok(doc.gesture_cancel().then_some(())))
            .unwrap();
        assert!(!session.state().dirty, "cancel returns to the saved base");
    }

    #[test]
    fn a_save_that_finishes_after_a_replace_marks_nothing() {
        let mut session = Session::new(Project::new("First"));
        let saved = session.save_snapshot();
        session.replace(Project::new("Second"), None, CURRENT_SCHEMA_VERSION);
        rename(&mut session, "Edited");
        let revision = session.state().revision;

        assert!(!session.mark_saved(&saved, PathBuf::from("/p/first.cutframe")));
        let state = session.state();
        assert!(state.dirty);
        assert_eq!(state.file_path, None);
        assert_eq!(state.revision, revision);
    }

    #[test]
    fn a_restored_session_is_dirty_until_saved() {
        let mut session = Session::new(Project::new("Test"));
        let original = PathBuf::from("/p/original.cutframe");
        session.replace_restored(Project::new("Restored"), Some(original.clone()), 0);
        let state = session.state();
        assert!(state.dirty);
        assert!(!state.history.can_undo && !state.history.can_redo);
        assert_eq!(state.file_path.as_deref(), Some("/p/original.cutframe"));
        assert_eq!(
            session.save_snapshot().backup_of_version(&original),
            Some(0),
            "the first save still backs up the older file"
        );

        rename(&mut session, "A");
        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert!(session.state().dirty, "undo does not make it clean");
        session.edit(|doc| Ok(doc.redo().then_some(()))).unwrap();
        assert!(session.state().dirty, "redo does not make it clean");
        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();

        save(&mut session, "/p/original.cutframe");
        assert!(!session.state().dirty);
        rename(&mut session, "B");
        session.edit(|doc| Ok(doc.undo().then_some(()))).unwrap();
        assert!(!session.state().dirty, "clean at the saved point again");
    }

    #[test]
    fn snapshot_key_and_dirty_follow_the_document() {
        let mut session = Session::new(Project::new("Test"));
        let clean = session.save_snapshot();
        assert!(!clean.dirty);
        rename(&mut session, "A");
        let edited = session.save_snapshot();
        assert!(edited.dirty);
        assert_ne!(edited.key(), clean.key());
        session
            .edit(|doc| {
                doc.gesture_update(Box::new(RenameProject::new("Live")))
                    .map(Some)
            })
            .unwrap();
        assert_eq!(
            session.save_snapshot().key(),
            edited.key(),
            "gesture keeps the base key"
        );
        session.replace(Project::new("Other"), None, CURRENT_SCHEMA_VERSION);
        assert_ne!(session.save_snapshot().key().0, edited.key().0);
    }

    #[test]
    fn backups_only_when_overwriting_an_older_version_file() {
        let mut session = Session::new(Project::new("Test"));
        let opened = PathBuf::from("/p/old.cutframe");
        session.replace(Project::new("Old"), Some(opened.clone()), 0);
        let saved = session.save_snapshot();
        assert_eq!(saved.backup_of_version(&opened), Some(0));
        assert_eq!(
            saved.backup_of_version(Path::new("/p/other.cutframe")),
            None
        );

        session.mark_saved(&saved, opened.clone());
        assert_eq!(session.save_snapshot().backup_of_version(&opened), None);

        session.replace(
            Project::new("Current"),
            Some(opened.clone()),
            CURRENT_SCHEMA_VERSION,
        );
        assert_eq!(session.save_snapshot().backup_of_version(&opened), None);
    }
}
