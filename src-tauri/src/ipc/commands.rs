//! Commands callable from the UI. Keep handlers thin: validate, delegate, return.

use app_core::command::{AddTrack, RemoveTrack, RenameProject, RenameTrack};
use app_core::{TrackId, TrackKind};
use tauri::{AppHandle, State};

use super::IpcError;
use super::types::{AppInfo, GpuInfoView, PreviewMode, PreviewRect, ProjectState};
use crate::state::AppState;

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

/// Returns `false` if there was nothing to undo.
#[tauri::command]
#[specta::specta]
pub fn undo(app: AppHandle, state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state
        .edit(&app, |doc| Ok(doc.undo().then_some(())))?
        .is_some())
}

/// Returns `false` if there was nothing to redo.
#[tauri::command]
#[specta::specta]
pub fn redo(app: AppHandle, state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state
        .edit(&app, |doc| Ok(doc.redo().then_some(())))?
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
