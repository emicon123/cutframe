//! Application state shared by commands, the media protocol and the preview.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use app_core::command::{ClipboardContent, CommandError, DeleteClips, PasteClips};
use app_core::serialization::CURRENT_SCHEMA_VERSION;
use app_core::{ClipId, Document, HistoryPosition, MAX_IPC_FLICKS, Project, Time};
use arc_swap::ArcSwap;
use engine::playback::{Playback, ShuttleKey};
use engine::transport::SeekMode;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::autosave::{AutosaveHandle, Msg};
use crate::export::{EncoderProbe, ExportSlot};
use crate::ipc::IpcError;
use crate::ipc::events::{PlaybackTick, ProjectChanged};
use crate::ipc::types::ProjectState;
use crate::ipc::types::{JobId, MediaStatus};
use crate::jobs::JobManager;
use crate::media_status::{self, MediaStatuses, StatusKey, StatusUpdate};
use crate::preferences::{Preferences, Startup};
use crate::preview::Preview;
use crate::previews::{self, Previews};
use crate::recent::{RecentEntry, RecentStore};
use crate::recovery::{RecoveryStore, Restored};

#[derive(Debug)]
pub struct AppState {
    session: Mutex<Session>,
    /// Latest project snapshot, shared with the engine (the preview thread). Lock-free reads;
    /// engine threads never take the document lock (ADR-0004 §2).
    snapshot: Arc<ArcSwap<Project>>,
    /// Serializes saves, so two saves never write the same file at once or mark clean out of order.
    save_lock: tauri::async_runtime::Mutex<()>,
    /// Set once in `setup` ([`AppState::with_preview`]); `None` without a GPU and in tests.
    pub preview: Option<Preview>,
    /// This process's recovery directory and session id.
    recovery: RecoveryStore,
    /// Set in `setup` once the autosave thread runs; tests run without it.
    autosave: OnceLock<AutosaveHandle>,
    /// Background work (import, previews, the encoder probe, export).
    pub jobs: JobManager,
    /// Posters, filmstrips and waveforms for the media panel (`media://`).
    pub previews: Arc<Previews>,
    preferences: PreferencesState,
    /// Project path from this process's command line, until the UI takes it.
    launch_path: Mutex<Option<PathBuf>>,
    /// Recent projects for the home screen (design `home-screen.md` §4).
    recent: RecentStore,
    /// Serializes every read-modify-write of the recent list.
    recent_lock: tauri::async_runtime::Mutex<()>,
    /// The running media check, cancelled when another project replaces the open one.
    media_check: Mutex<Option<JobId>>,
    /// The session's export encoder probe (export.md §4.4).
    pub encoder_probe: Arc<EncoderProbe>,
    /// The running export (one at a time, export.md §5.1).
    pub exports: Arc<ExportSlot>,
}

/// The preferences in effect and where they live (design `preferences.md` §4).
#[derive(Debug)]
pub struct PreferencesState {
    current: RwLock<Preferences>,
    /// `preferences.json` in the app config directory.
    path: PathBuf,
    /// The app's cache directory, used when `cacheDir` is not set.
    app_cache_dir: PathBuf,
    /// The effective cache root (`.../cutframe-cache`).
    cache_root: RwLock<PathBuf>,
    /// The file came from another schema version: the next write keeps it as a backup.
    backup_of_version: Mutex<Option<u32>>,
    /// Serializes writes of the file and changes of the cache root.
    write_lock: tauri::async_runtime::Mutex<()>,
}

impl PreferencesState {
    pub fn new(startup: Startup, path: PathBuf, app_cache_dir: PathBuf) -> Self {
        let cache_root = startup.preferences.cache_root(&app_cache_dir);
        Self {
            current: RwLock::new(startup.preferences),
            path,
            app_cache_dir,
            cache_root: RwLock::new(cache_root),
            backup_of_version: Mutex::new(startup.backup_of_version),
            write_lock: tauri::async_runtime::Mutex::new(()),
        }
    }

    pub fn get(&self) -> Preferences {
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn app_cache_dir(&self) -> &Path {
        &self.app_cache_dir
    }

    pub fn cache_root(&self) -> PathBuf {
        self.cache_root
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Hold this while writing the file or using the cache root.
    pub fn write_lock(&self) -> &tauri::async_runtime::Mutex<()> {
        &self.write_lock
    }

    /// The backup to make on the next write (see [`crate::preferences::write`]).
    pub fn backup_of_version(&self) -> Option<u32> {
        *self
            .backup_of_version
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records that `preferences` were written and the cache root exists. Returns the previous
    /// preferences.
    pub fn replace(&self, preferences: Preferences, cache_root: PathBuf) -> Preferences {
        *self
            .backup_of_version
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .cache_root
            .write()
            .unwrap_or_else(PoisonError::into_inner) = cache_root;
        std::mem::replace(
            &mut *self.current.write().unwrap_or_else(PoisonError::into_inner),
            preferences,
        )
    }
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
        recovery: RecoveryStore,
        jobs: JobManager,
        previews: Arc<Previews>,
    ) -> Self {
        let session = Session::new(project);
        let snapshot = Arc::new(ArcSwap::new(session.document.snapshot()));
        Self {
            session: Mutex::new(session),
            snapshot,
            save_lock: tauri::async_runtime::Mutex::new(()),
            preview: None,
            recovery,
            autosave: OnceLock::new(),
            jobs,
            previews,
            preferences: PreferencesState::new(
                Startup {
                    preferences: Preferences::default(),
                    backup_of_version: None,
                },
                PathBuf::new(),
                PathBuf::new(),
            ),
            launch_path: Mutex::new(None),
            recent: RecentStore::new(Path::new("")),
            recent_lock: tauri::async_runtime::Mutex::new(()),
            media_check: Mutex::new(None),
            encoder_probe: Arc::default(),
            exports: Arc::default(),
        }
    }

    /// Attaches the preview, which must have been created with [`AppState::shared_snapshot`].
    pub fn with_preview(mut self, preview: Option<Preview>) -> Self {
        self.preview = preview;
        self
    }

    /// The snapshot the engine reads; [`AppState::publish`] stores every new project into it.
    pub fn shared_snapshot(&self) -> Arc<ArcSwap<Project>> {
        Arc::clone(&self.snapshot)
    }

    /// Keeps the recent list in `store` instead of the working directory.
    pub fn with_recent(mut self, store: RecentStore) -> Self {
        self.recent = store;
        self
    }

    pub fn recent(&self) -> &RecentStore {
        &self.recent
    }

    /// Hold this while using [`AppState::recent`].
    pub fn recent_lock(&self) -> &tauri::async_runtime::Mutex<()> {
        &self.recent_lock
    }

    /// Records an open or save of `path` in the recent list. A failure is only logged: it never
    /// fails the open or save.
    pub async fn record_recent(&self, entry: RecentEntry) {
        let _recording = self.recent_lock.lock().await;
        let store = self.recent.clone();
        let recorded = tauri::async_runtime::spawn_blocking(move || store.record(entry)).await;
        match recorded {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!(%err, "cannot record the recent project"),
            Err(err) => tracing::warn!(%err, "cannot record the recent project"),
        }
    }

    /// The open project's file, if it has one.
    pub fn open_file(&self) -> Option<PathBuf> {
        self.lock().file_path.clone()
    }

    /// Uses the preferences read at startup instead of the defaults.
    pub fn with_preferences(mut self, preferences: PreferencesState) -> Self {
        self.preferences = preferences;
        self
    }

    /// Records the project path from the command line for [`AppState::take_launch_path`].
    pub fn with_launch_path(self, path: Option<PathBuf>) -> Self {
        *self
            .launch_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = path;
        self
    }

    pub fn preferences(&self) -> &PreferencesState {
        &self.preferences
    }

    /// The command-line project path, once; later calls return `None`.
    pub fn take_launch_path(&self) -> Option<PathBuf> {
        self.launch_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Tells the autosave thread the new interval (`None` = off, which also deletes the own
    /// autosave).
    pub fn set_autosave_interval(&self, interval: Option<Duration>) {
        self.notify_autosave(Msg::SetInterval(interval));
    }

    /// Starts the background preview job for `ids` (after an import), or for every asset of the
    /// open project (`None`, after open).
    pub fn prefetch_previews(&self, ids: Option<Vec<app_core::MediaId>>) {
        previews::start_prefetch(&self.jobs, Arc::clone(&self.previews), ids);
    }

    /// Starts the background media check (`checkingMedia`) for the open project, cancelling one
    /// still running for a project opened before.
    pub fn check_media(&self) {
        let id = media_status::start_check(&self.jobs);
        let previous = self
            .media_check
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(id);
        if let Some(previous) = previous {
            self.jobs.cancel(previous);
        }
    }

    /// Records media statuses (design §6.6). Bumps the revision and emits `projectChanged` only
    /// when one changed.
    pub fn set_media_status(
        &self,
        app: &AppHandle,
        updates: Vec<StatusUpdate>,
    ) -> Result<(), IpcError> {
        let state = {
            let mut session = self.lock();
            if !session.set_media_status(updates) {
                return Ok(());
            }
            session.state()
        };
        ProjectChanged(state).emit(app)?;
        Ok(())
    }

    /// A decoder or preview generator found `asset`'s file gone: it is `offline` from now on.
    pub fn report_missing(&self, app: &AppHandle, asset: &app_core::MediaAsset) {
        let update = (StatusKey::of(asset), MediaStatus::Offline);
        if let Err(err) = self.set_media_status(app, vec![update]) {
            tracing::warn!(%err, "failed to emit project-changed for offline media");
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
        self.edit_session(app, |session| session.edit(edit).map_err(IpcError::from))
    }

    /// Copies `clips` and their linked members to the clip clipboard (design §6.3). Not an edit:
    /// no history entry, no event.
    pub fn copy_clips(&self, clips: &[ClipId]) -> Result<(), IpcError> {
        self.lock().copy(clips)
    }

    /// Copies `clips` to the clipboard and deletes them, as one history entry (`cutClips`).
    pub fn cut_clips(&self, app: &AppHandle, clips: Vec<ClipId>) -> Result<(), IpcError> {
        self.edit_session(app, |session| session.cut(clips))?;
        Ok(())
    }

    /// Pastes the clipboard at `at` (`pasteClips`) and returns the new clips' ids.
    pub fn paste_clips(&self, app: &AppHandle, at: Time) -> Result<Vec<ClipId>, IpcError> {
        Ok(self
            .edit_session(app, |session| session.paste(at))?
            .unwrap_or_default())
    }

    /// Runs `edit` on the session; publishes and notifies like [`AppState::edit`].
    fn edit_session<T>(
        &self,
        app: &AppHandle,
        edit: impl FnOnce(&mut Session) -> Result<Option<T>, IpcError>,
    ) -> Result<Option<T>, IpcError> {
        let (result, state) = {
            let mut session = self.lock();
            let Some(result) = edit(&mut session)? else {
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
        self.reset_transport();
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
        self.reset_transport();
        self.notify_autosave(Msg::Adopt { old });
        ProjectChanged(state).emit(app)?;
        Ok(())
    }

    /// Hands the document's current state to the engine: stores the snapshot, then wakes the
    /// preview thread (render on change). Never blocks.
    fn publish(&self, document: &Document) {
        self.snapshot.store(document.snapshot());
        if let Some(preview) = &self.preview {
            preview.snapshot_changed();
        }
    }

    /// Moves the playhead (the `seek` command). Returns the new seek generation. `time` outside
    /// ±2^53 flicks is `invalidEdit`; no preview engine is `unavailable`.
    pub fn seek(&self, time: Time, mode: SeekMode) -> Result<u32, IpcError> {
        if time.flicks().unsigned_abs() > MAX_IPC_FLICKS.unsigned_abs() {
            return Err(IpcError::InvalidEdit(format!(
                "seek time {} is outside ±2^53 flicks",
                time.flicks()
            )));
        }
        Ok(self.playback()?.seek(time, mode))
    }

    /// The transport of the preview engine; `unavailable` without one.
    fn playback(&self) -> Result<&std::sync::Arc<Playback>, IpcError> {
        match &self.preview {
            Some(preview) if preview.is_available() => Ok(preview.playback()),
            _ => Err(IpcError::Unavailable("no preview engine".into())),
        }
    }

    /// Play at 1x or pause (`toggle_playback`).
    pub fn toggle_playback(&self) -> Result<(), IpcError> {
        self.playback()?.toggle();
        Ok(())
    }

    pub fn pause_playback(&self) -> Result<(), IpcError> {
        self.playback()?.pause();
        Ok(())
    }

    /// J/K/L (`shuttle`).
    pub fn shuttle(&self, key: ShuttleKey) -> Result<(), IpcError> {
        self.playback()?.shuttle(key);
        Ok(())
    }

    /// Pause and move whole frames (`step_frames`). Returns the new seek generation.
    pub fn step_frames(&self, frames: i32) -> Result<u32, IpcError> {
        Ok(self.playback()?.step(frames))
    }

    /// Loop on or off (`set_loop`); without a preview engine nothing happens.
    pub fn set_loop(&self, on: bool) {
        if let Ok(playback) = self.playback() {
            playback.set_loop(on);
        }
    }

    /// The transport now (`get_playback`).
    pub fn playback_tick(&self) -> PlaybackTick {
        self.playback()
            .map_or(PlaybackTick::IDLE, |playback| playback.status().into())
    }

    /// A new, opened or restored project: pause, playhead 0, a new seek generation.
    fn reset_transport(&self) {
        if let Ok(playback) = self.playback() {
            playback.reset();
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

    /// The document's current snapshot (tests publish it by hand).
    #[cfg(test)]
    fn session_snapshot(&self) -> Arc<Project> {
        self.lock().document.snapshot()
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
    /// Whether each media file is reachable (design §6.6). Not part of the document.
    media_status: MediaStatuses,
    /// Copied clips (design §6.3). Media ids belong to a project, so replacing the document
    /// clears it.
    clipboard: Option<ClipboardContent>,
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
            media_status: MediaStatuses::default(),
            clipboard: None,
        }
    }

    fn state(&self) -> ProjectState {
        ProjectState::new(
            &self.document,
            self.revision,
            self.file_path.as_deref(),
            self.dirty(),
            &self.media_status,
        )
    }

    /// Bumps the revision if a status changed.
    fn set_media_status(&mut self, updates: Vec<StatusUpdate>) -> bool {
        let changed = self.media_status.apply(updates);
        if changed {
            self.revision += 1;
        }
        changed
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

    fn copy(&mut self, clips: &[ClipId]) -> Result<(), IpcError> {
        self.clipboard = Some(ClipboardContent::copy(self.document.project(), clips)?);
        Ok(())
    }

    /// Copy, then a delete labelled `cutClips`; the clipboard changes only if the delete works.
    fn cut(&mut self, clips: Vec<ClipId>) -> Result<Option<()>, IpcError> {
        let content = ClipboardContent::copy(self.document.project(), &clips)?;
        self.edit(|doc| doc.execute(Box::new(DeleteClips::cut(clips))).map(Some))?;
        self.clipboard = Some(content);
        Ok(Some(()))
    }

    fn paste(&mut self, at: Time) -> Result<Option<Vec<ClipId>>, IpcError> {
        let content = self
            .clipboard
            .clone()
            .ok_or_else(|| IpcError::InvalidEdit("the clipboard is empty".into()))?;
        let command = PasteClips::new(content, at);
        let ids = command.clip_ids().to_vec();
        self.edit(|doc| doc.execute(Box::new(command)).map(Some))?;
        Ok(Some(ids))
    }

    fn replace(&mut self, project: Project, file_path: Option<PathBuf>, source_version: u32) {
        self.document = Document::new(project);
        self.saved_at = Some(self.document.position());
        self.file_path = file_path;
        self.source_version = source_version;
        self.generation += 1;
        self.revision += 1;
        // Until the check of the new project says otherwise, its media counts as online.
        self.media_status.clear();
        self.clipboard = None;
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
    use engine::preview::{PreviewHandle, Wake, WakeProbe};

    use super::*;

    fn rename(session: &mut Session, name: &str) {
        session
            .edit(|doc| doc.execute(Box::new(RenameProject::new(name))).map(Some))
            .expect("rename");
    }

    fn app_state() -> AppState {
        let recovery = crate::recovery::RecoveryStore::new(PathBuf::new(), uuid::Uuid::now_v7());
        AppState::new(
            Project::new("Test"),
            recovery,
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        )
    }

    #[test]
    fn the_launch_path_is_taken_once() {
        let path = PathBuf::from("/p/launched.cutframe");
        let state = app_state().with_launch_path(Some(path.clone()));
        assert_eq!(state.take_launch_path(), Some(path));
        assert_eq!(state.take_launch_path(), None);
        assert_eq!(app_state().take_launch_path(), None);
    }

    #[test]
    fn replacing_preferences_clears_the_pending_backup() {
        let app_cache = std::env::temp_dir();
        let prefs = PreferencesState::new(
            Startup {
                preferences: Preferences::default(),
                backup_of_version: Some(7),
            },
            PathBuf::from("preferences.json"),
            app_cache.clone(),
        );
        assert_eq!(
            prefs.cache_root(),
            Preferences::default().cache_root(&app_cache)
        );
        assert_eq!(prefs.backup_of_version(), Some(7));
        let next = Preferences {
            autosave_interval: None,
            ..Preferences::default()
        };
        let previous = prefs.replace(next.clone(), PathBuf::from("/c/cutframe-cache"));
        assert_eq!(previous, Preferences::default());
        assert_eq!(prefs.get(), next);
        assert_eq!(prefs.cache_root(), PathBuf::from("/c/cutframe-cache"));
        assert_eq!(prefs.backup_of_version(), None);
    }

    #[test]
    fn media_status_bumps_the_revision_and_is_cleared_by_replace() {
        use app_core::media::samples;
        let mut project = Project::new("Test");
        project.media.push(samples::video(1));
        let asset = project.media[0].clone();
        let mut session = Session::new(project.clone());
        let offline = || vec![(StatusKey::of(&asset), MediaStatus::Offline)];

        assert_eq!(session.state().project.media[0].status, MediaStatus::Online);
        assert!(session.set_media_status(offline()));
        assert_eq!(session.state().revision, 1);
        assert_eq!(
            session.state().project.media[0].status,
            MediaStatus::Offline
        );
        assert!(!session.dirty(), "status is not an edit");
        assert!(!session.set_media_status(offline()), "no change");
        assert_eq!(session.state().revision, 1);

        session.replace(project, None, CURRENT_SCHEMA_VERSION);
        assert_eq!(session.state().project.media[0].status, MediaStatus::Online);
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

    /// An `AppState` whose preview has no render thread: the probe sees what it would receive.
    fn app_state_with_preview() -> (AppState, WakeProbe) {
        let state = app_state();
        let (engine, probe) = PreviewHandle::detached(state.shared_snapshot());
        (state.with_preview(Some(Preview::detached(engine))), probe)
    }

    const WAIT: Duration = Duration::from_millis(200);

    #[test]
    fn publishing_stores_the_snapshot_then_wakes_the_render_thread() {
        let (state, probe) = app_state_with_preview();
        let engine_snapshot = state.shared_snapshot();
        state.test_edit(|doc| {
            doc.execute(Box::new(RenameProject::new("Renamed")))
                .map(Some)
        });
        assert_eq!(probe.next(WAIT), Some(Wake::SnapshotChanged));
        // The wake follows the store, so the engine reads the new project.
        assert_eq!(engine_snapshot.load().name, "Renamed");
        assert_eq!(probe.next(WAIT), None, "one wake per publish");
    }

    #[test]
    fn seek_returns_increasing_generations_and_wakes_the_preview() {
        let (state, probe) = app_state_with_preview();
        assert_eq!(state.seek(Time::from_seconds(3), SeekMode::Scrub), Ok(1));
        assert_eq!(
            probe.next(WAIT),
            Some(Wake::Seek {
                generation: 1,
                mode: SeekMode::Scrub
            })
        );
        assert_eq!(state.seek(Time::ZERO, SeekMode::Exact), Ok(2));
        assert_eq!(state.seek(Time::ZERO, SeekMode::Exact), Ok(3));
        assert_eq!(
            probe.next(WAIT),
            Some(Wake::Seek {
                generation: 2,
                mode: SeekMode::Exact
            })
        );
    }

    #[test]
    fn seek_rejects_times_outside_the_ipc_range() {
        let (state, probe) = app_state_with_preview();
        for flicks in [MAX_IPC_FLICKS + 1, -MAX_IPC_FLICKS - 1, i64::MAX, i64::MIN] {
            assert!(
                matches!(
                    state.seek(Time(flicks), SeekMode::Exact),
                    Err(IpcError::InvalidEdit(_))
                ),
                "{flicks}"
            );
        }
        assert_eq!(probe.next(WAIT), None, "a rejected seek changes nothing");
        // The bounds themselves are accepted (and clamped to the timeline).
        assert_eq!(state.seek(Time(MAX_IPC_FLICKS), SeekMode::Exact), Ok(1));
        assert_eq!(state.seek(Time(-MAX_IPC_FLICKS), SeekMode::Exact), Ok(2));
    }

    #[test]
    fn seek_without_a_preview_engine_is_unavailable() {
        assert!(matches!(
            app_state().seek(Time::ZERO, SeekMode::Exact),
            Err(IpcError::Unavailable(_))
        ));
    }

    #[test]
    fn transport_commands_without_a_preview_engine_are_unavailable() {
        let state = app_state();
        let unavailable = |result: Result<(), IpcError>| {
            assert!(
                matches!(result, Err(IpcError::Unavailable(_))),
                "{result:?}"
            );
        };
        unavailable(state.toggle_playback());
        unavailable(state.pause_playback());
        unavailable(state.shuttle(ShuttleKey::Forward));
        unavailable(state.step_frames(1).map(|_| ()));
        state.set_loop(true);
        assert_eq!(state.playback_tick(), PlaybackTick::IDLE);
    }

    /// A project of `seconds` of colour on the main track.
    fn timeline_of(seconds: i64) -> Project {
        let mut project = Project::new("Timeline");
        let main = project.main_index().unwrap();
        project.tracks[main].clips.push(app_core::Clip::color(
            "C",
            Time::ZERO,
            Time::from_seconds(seconds),
            app_core::Rgba8::BLACK,
        ));
        project
    }

    #[test]
    fn transport_commands_drive_the_engine_and_report_through_ticks() {
        use crate::ipc::types::TransportStateView;
        let (state, probe) = app_state_with_preview();
        state.test_replace(timeline_of(10), None, CURRENT_SCHEMA_VERSION);
        state.snapshot.store(state.session_snapshot());

        state.toggle_playback().unwrap();
        assert_eq!(probe.next(WAIT), Some(Wake::Transport));
        let tick = state.playback_tick();
        assert_eq!(tick.state, TransportStateView::Playing);
        assert_eq!(tick.rate, 1);

        state.shuttle(ShuttleKey::Forward).unwrap();
        assert_eq!(state.playback_tick().rate, 2);
        state.set_loop(true);
        assert!(state.playback_tick().looping);

        let before = state.playback_tick().seek;
        let generation = state.step_frames(3).unwrap();
        assert!(generation > before);
        let tick = state.playback_tick();
        assert_eq!(tick.state, TransportStateView::Paused);
        assert_eq!(tick.seek, generation);
        assert_eq!(tick.time, Time(3 * app_core::FLICKS_PER_SECOND / 30));
        state.pause_playback().unwrap();
        assert!(state.seek(Time::from_seconds(5), SeekMode::Exact).unwrap() > generation);
    }

    #[test]
    fn replacing_the_project_resets_the_transport() {
        let (state, probe) = app_state_with_preview();
        state.test_replace(timeline_of(10), None, CURRENT_SCHEMA_VERSION);
        state.snapshot.store(state.session_snapshot());
        state.seek(Time::from_seconds(4), SeekMode::Exact).unwrap();
        state.toggle_playback().unwrap();
        while probe.next(WAIT).is_some() {}
        let before = state.playback_tick().seek;
        state.reset_transport();
        let tick = state.playback_tick();
        assert_eq!(tick.time, Time::ZERO);
        assert_eq!(tick.state, crate::ipc::types::TransportStateView::Paused);
        assert_eq!(tick.seek, before + 1);
        assert_eq!(
            probe.next(WAIT),
            Some(Wake::Seek {
                generation: before + 1,
                mode: SeekMode::Exact
            })
        );
    }

    /// A project with two colour cards on the main track.
    fn with_clips() -> (Session, [ClipId; 2]) {
        use app_core::command::{AddClips, NewClipSource, Placement};
        let mut session = Session::new(Project::new("Clips"));
        let color = NewClipSource::Color {
            color: app_core::Rgba8::BLACK,
        };
        let add = AddClips::new(vec![color, color], Placement::Append);
        let ids = [add.clip_ids()[0], add.clip_ids()[1]];
        session
            .edit(|doc| doc.execute(Box::new(add)).map(Some))
            .unwrap();
        (session, ids)
    }

    fn main_ids(session: &Session) -> Vec<ClipId> {
        let project = session.document.project();
        project.main().unwrap().clips.iter().map(|c| c.id).collect()
    }

    #[test]
    fn copy_then_paste_adds_new_clips_and_keeps_the_clipboard() {
        let (mut session, [a, b]) = with_clips();
        let revision = session.revision;
        session.copy(&[a]).unwrap();
        assert_eq!(session.revision, revision, "copy is not an edit");
        assert_eq!(session.document.history_len(), 1);

        let pasted = session.paste(Time::ZERO).unwrap().unwrap();
        assert_eq!(pasted.len(), 1);
        assert_eq!(main_ids(&session), vec![pasted[0], a, b]);
        assert_eq!(
            session.document.undo_label(),
            Some(HistoryLabel::PasteClips)
        );
        let again = session.paste(Time::ZERO).unwrap().unwrap();
        assert_ne!(again, pasted, "every paste makes new clips");
    }

    #[test]
    fn cut_deletes_in_one_entry_and_fills_the_clipboard() {
        let (mut session, [a, b]) = with_clips();
        session.cut(vec![a]).unwrap();
        assert_eq!(main_ids(&session), vec![b]);
        assert_eq!(session.document.undo_label(), Some(HistoryLabel::CutClips));
        assert_eq!(session.document.history_len(), 2);
        let pasted = session.paste(Time::ZERO).unwrap().unwrap();
        assert_eq!(main_ids(&session), vec![pasted[0], b]);
    }

    #[test]
    fn a_failed_cut_leaves_the_clipboard_alone() {
        let (mut session, [a, _]) = with_clips();
        session.copy(&[a]).unwrap();
        let copied = session.clipboard.clone();
        assert!(session.cut(vec![ClipId::new()]).is_err());
        assert_eq!(session.clipboard, copied);
    }

    #[test]
    fn paste_with_an_empty_clipboard_is_an_invalid_edit() {
        let (mut session, _) = with_clips();
        assert!(matches!(
            session.paste(Time::ZERO),
            Err(IpcError::InvalidEdit(_))
        ));
    }

    #[test]
    fn new_open_and_restore_clear_the_clipboard() {
        let (mut session, [a, _]) = with_clips();
        session.copy(&[a]).unwrap();
        session.replace(Project::new("New"), None, CURRENT_SCHEMA_VERSION);
        assert!(session.clipboard.is_none());
        assert!(session.paste(Time::ZERO).is_err());

        let (mut session, [a, _]) = with_clips();
        session.copy(&[a]).unwrap();
        session.replace_restored(Project::new("Restored"), None, CURRENT_SCHEMA_VERSION);
        assert!(session.clipboard.is_none());
    }
}
