//! Commands callable from the UI. Keep handlers thin: validate, delegate, return.

use std::path::PathBuf;

use app_core::command::{AddTrack, RemoveTrack, RenameProject, RenameTrack, SetProjectSettings};
use app_core::serialization::{self, CURRENT_SCHEMA_VERSION};
use app_core::{Project, ProjectSettings, TrackId, TrackKind};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use super::IpcError;
use super::types::{AppInfo, GpuInfoView, PreviewMode, PreviewRect, ProjectState, RecoveryEntry};
use crate::project_file;
use crate::state::{AppState, SaveSnapshot};

/// Name of a project made by `new_project` and of the one open at startup.
pub const DEFAULT_PROJECT_NAME: &str = "Untitled project";

/// Current project state. Call once on startup; afterwards listen to `projectChanged`.
#[tauri::command]
#[specta::specta]
pub fn get_project(state: State<'_, AppState>) -> ProjectState {
    state.project_state()
}

/// Adds an empty track at `index` (top = 0) or at the bottom. Returns the new track's id.
#[tauri::command]
#[specta::specta]
pub fn add_track(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: TrackKind,
    index: Option<u32>,
) -> Result<TrackId, IpcError> {
    let command = AddTrack::new(kind, index.map(|i| i as usize));
    let id = command.track_id();
    state.edit(&app, |doc| doc.execute(Box::new(command)).map(Some))?;
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub fn remove_track(
    app: AppHandle,
    state: State<'_, AppState>,
    id: TrackId,
) -> Result<(), IpcError> {
    state.edit(&app, |doc| {
        doc.execute(Box::new(RemoveTrack::new(id))).map(Some)
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn rename_track(
    app: AppHandle,
    state: State<'_, AppState>,
    id: TrackId,
    name: String,
) -> Result<(), IpcError> {
    state.edit(&app, |doc| {
        doc.execute(Box::new(RenameTrack::new(id, name))).map(Some)
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn rename_project(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), IpcError> {
    state.edit(&app, |doc| {
        doc.execute(Box::new(RenameProject::new(name))).map(Some)
    })?;
    Ok(())
}

/// Cancels the open gesture (ADR-0006) and restores the state before it; records nothing. Call it
/// on Escape, lost pointer capture, window blur and at startup. Does nothing if no gesture is open.
#[tauri::command]
#[specta::specta]
pub fn cancel_gesture(app: AppHandle, state: State<'_, AppState>) -> Result<(), IpcError> {
    state.edit(&app, |doc| Ok(doc.gesture_cancel().then_some(())))?;
    Ok(())
}

/// Returns `false` if there was nothing to undo. While a gesture is open, cancels it instead.
#[tauri::command]
#[specta::specta]
pub fn undo(app: AppHandle, state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state
        .edit(&app, |doc| Ok(doc.undo().then_some(())))?
        .is_some())
}

/// Returns `false` if there was nothing to redo. While a gesture is open, cancels it instead.
#[tauri::command]
#[specta::specta]
pub fn redo(app: AppHandle, state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state
        .edit(&app, |doc| Ok(doc.redo().then_some(())))?
        .is_some())
}

/// Moves through the history until `index` entries are applied (`history.current` becomes
/// `index`): the same as that many undo or redo steps, with one `projectChanged`. An open gesture
/// is cancelled first. Returns `false` if nothing changed (already there). Fails with
/// `invalidEdit` if `index > history.entries.length`; then nothing changes.
#[tauri::command]
#[specta::specta]
pub fn jump_to_history(
    app: AppHandle,
    state: State<'_, AppState>,
    index: u32,
) -> Result<bool, IpcError> {
    Ok(state
        .edit(&app, |doc| Ok(doc.jump_to(index as usize)?.then_some(())))?
        .is_some())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        gpu: state.preview.as_ref().map(|p| {
            let info = p.gpu_info();
            GpuInfoView {
                adapter: info.adapter.clone(),
                backend: info.backend.clone(),
                software: info.software,
            }
        }),
        preview_mode: state
            .preview
            .as_ref()
            .map_or(PreviewMode::Unavailable, |p| p.mode()),
    }
}

/// Reports where the preview area is. In `native` mode the GPU window follows this rectangle.
/// An empty rectangle hides the preview; a non-empty one never shows a preview that
/// `set_preview_visible(false)` hid.
#[tauri::command]
#[specta::specta]
pub fn set_preview_rect(app: AppHandle, state: State<'_, AppState>, rect: PreviewRect) {
    if let Some(preview) = &state.preview {
        preview.set_rect(&app, rect);
    }
}

/// Requests the preview shown (`true`) or hidden (`false`). The preview starts hidden: request
/// visible when the preview area mounts and hidden when it unmounts or a dialog overlaps it. It is
/// on screen only while requested and the last `set_preview_rect` was non-empty.
#[tauri::command]
#[specta::specta]
pub fn set_preview_visible(app: AppHandle, state: State<'_, AppState>, visible: bool) {
    if let Some(preview) = &state.preview {
        preview.set_visible(&app, visible);
    }
}

/// Replaces the open project with an empty one (default tracks) with `settings`, or the default
/// settings. The history is cleared and the project is clean. Ask about unsaved changes first.
#[tauri::command]
#[specta::specta]
pub fn new_project(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Option<ProjectSettings>,
) -> Result<(), IpcError> {
    let mut project = Project::new(DEFAULT_PROJECT_NAME);
    if let Some(settings) = settings {
        settings
            .validate()
            .map_err(|err| IpcError::InvalidEdit(err.to_string()))?;
        project.settings = settings;
    }
    state.replace_project(&app, project, None, CURRENT_SCHEMA_VERSION)
}

/// Opens the project file at `path` (absolute) in place of the open project. The history is
/// cleared and the project is clean. The extension is not checked: the content decides. On error
/// the open project is unchanged. Ask about unsaved changes first.
#[tauri::command]
#[specta::specta]
pub async fn open_project(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), IpcError> {
    let path = std::path::absolute(PathBuf::from(path)).map_err(|err| IpcError::io(&err))?;
    let (path, loaded) = tauri::async_runtime::spawn_blocking(move || {
        let loaded = project_file::read_project_file(&path);
        (path, loaded)
    })
    .await?;
    let loaded = loaded?;
    state.replace_project(&app, loaded.project, Some(path), loaded.source_version)
}

/// Saves to the open project's file. Returns `false` if the project has no file yet: nothing was
/// written and the UI runs Save As. Not an undo step.
#[tauri::command]
#[specta::specta]
pub async fn save_project(app: AppHandle, state: State<'_, AppState>) -> Result<bool, IpcError> {
    let _saving = state.save_lock().lock().await;
    let snapshot = state.save_snapshot();
    let Some(path) = snapshot.file_path.clone() else {
        return Ok(false);
    };
    write(&app, &state, snapshot, path).await?;
    Ok(true)
}

/// Saves to `path` (absolute; `.cutframe` is appended when it has no extension) and makes it the
/// project's file. Not an undo step.
#[tauri::command]
#[specta::specta]
pub async fn save_project_as(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), IpcError> {
    let path = std::path::absolute(PathBuf::from(path)).map_err(|err| IpcError::io(&err))?;
    let path = project_file::with_project_extension(path);
    let _saving = state.save_lock().lock().await;
    let snapshot = state.save_snapshot();
    write(&app, &state, snapshot, path).await
}

/// Serializes and writes `snapshot` off the main thread, then marks it saved.
async fn write(
    app: &AppHandle,
    state: &AppState,
    snapshot: SaveSnapshot,
    path: PathBuf,
) -> Result<(), IpcError> {
    let backup = snapshot.backup_of_version(&path);
    let project = snapshot.project.clone();
    let (path, written) = tauri::async_runtime::spawn_blocking(move || {
        let json = serialization::to_json(&project);
        let written = project_file::write_project_file(&path, &json, backup);
        (path, written)
    })
    .await?;
    written.map_err(|err| IpcError::io(&err))?;
    state.mark_saved(app, &snapshot, path)
}

/// Changes canvas size, frame rate and sample rate as one undoable entry. Clip times are kept
/// exactly. Settings equal to the current ones change nothing and record nothing. Out-of-range
/// settings fail with `invalidEdit`.
#[tauri::command]
#[specta::specta]
pub fn set_project_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: ProjectSettings,
) -> Result<(), IpcError> {
    state.edit(&app, |doc| {
        if doc.project().settings == settings {
            return Ok(None);
        }
        doc.execute(Box::new(SetProjectSettings::new(settings)))
            .map(Some)
    })?;
    Ok(())
}

/// Autosaves left by instances that did not exit cleanly and that are newer than their project
/// file, newest first. Prunes corrupt and stale entries as a side effect. A missing recovery
/// directory gives an empty list. Call once at startup.
#[tauri::command]
#[specta::specta]
pub async fn list_recovery(state: State<'_, AppState>) -> Result<Vec<RecoveryEntry>, IpcError> {
    let store = state.recovery().clone();
    let entries = tauri::async_runtime::spawn_blocking(move || store.list_orphans())
        .await?
        .map_err(|err| IpcError::io(&err))?;
    Ok(entries.into_iter().map(RecoveryEntry::from).collect())
}

/// Opens recovery entry `id` in place of the open project: the history is empty, the project is
/// dirty until saved, and `filePath` is the original file (or none). Emits `projectChanged`. Fails
/// with `fileNotFound` when no such entry can be restored (gone, or owned by a running instance).
/// Ask about unsaved changes first.
#[tauri::command]
#[specta::specta]
pub async fn restore_recovery(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<(), IpcError> {
    let store = state.recovery().clone();
    let restored = tauri::async_runtime::spawn_blocking(move || store.load(id)).await??;
    state.replace_restored(&app, restored, id)
}

/// Deletes recovery entry `id`. Unknown or already deleted ids are fine; an entry of a running
/// instance is never touched.
#[tauri::command]
#[specta::specta]
pub async fn discard_recovery(state: State<'_, AppState>, id: Uuid) -> Result<(), IpcError> {
    let store = state.recovery().clone();
    tauri::async_runtime::spawn_blocking(move || store.discard(id))
        .await?
        .map_err(|err| IpcError::io(&err))
}

/// Closes the main window without asking again. Call it after the unsaved-changes prompt that
/// `quitRequested` started (Save succeeded, or Don't save).
#[tauri::command]
#[specta::specta]
pub fn confirm_close(app: AppHandle) -> Result<(), IpcError> {
    let window = app
        .get_webview_window(crate::MAIN_WINDOW)
        .ok_or_else(|| IpcError::Internal("main window not found".into()))?;
    // `destroy` does not raise `CloseRequested` again.
    window.destroy()?;
    Ok(())
}
