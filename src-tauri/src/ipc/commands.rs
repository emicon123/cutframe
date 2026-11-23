//! Commands callable from the UI. Keep handlers thin: validate, delegate, return.

use std::path::{Path, PathBuf};

use app_core::command::{
    AddClips, AddTrack, Batch, Command, DeleteClips, DeleteGap, Edge, HistoryLabel, MoveClips,
    NewClipSource, PasteClips, Placement, RemoveClipsOfMedia, RemoveMedia, RemoveTrack,
    RenameProject, RenameTrack, SetMagnetic, SetProjectSettings, Side, SplitClips, TrackTarget,
    TrimClip, TrimToPlayhead,
};
use app_core::serialization::{self, CURRENT_SCHEMA_VERSION};
use app_core::{
    ClipId, EditPhase, MAX_IPC_FLICKS, MediaId, Project, ProjectSettings, Time, TrackId, TrackKind,
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use super::IpcError;
use super::types::{
    AppInfo, CacheInfo, JobId, PreviewMode, PreviewRect, ProjectState, RecentProject,
    RecoveryEntry, SeekMode,
};
use crate::import;
use crate::media_paths;
use crate::preferences::{self, CURRENT_PREFERENCES_VERSION, Preferences, PreferencesError};
use crate::project_file;
use crate::recent::{self, RecentEntry, RecentStore};
use crate::relink;
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

/// Puts media items or colour cards on the timeline as one undo step (`addClips`) and returns the
/// new clips' ids, one per source in order (select them). Several sources go one after another.
///
/// - `append`: end of the main track; audio-only media at the end of the first unlocked audio
///   track (created at the top of the audio section when there is none).
/// - `insert`: into the main track at the clip boundary nearest `at`; later main clips ripple and
///   the overlays, captions and audio over them follow (bumped to another track on collision).
/// - `drop`: on an existing track or a new one (`new.index` is clamped into the section that fits
///   the first clip) from `start`. On the magnetic main track a drop is an insert.
///
/// Video clips take the media's whole length (with its sound when it has audio), images 5 s and
/// colour cards 5 s. Fails with `overlap` for an occupied range, `trackLocked` for a locked
/// target, and `invalidEdit` for no sources, unknown media or tracks, a clip that does not fit the
/// track (e.g. audio-only media on a video track), or a time outside `0..=2^53` flicks.
#[tauri::command]
#[specta::specta]
pub fn add_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    sources: Vec<NewClipSource>,
    placement: Placement,
) -> Result<Vec<ClipId>, IpcError> {
    if sources.is_empty() {
        return Err(IpcError::InvalidEdit("no clips to add".into()));
    }
    let time = match placement {
        Placement::Append => Time::ZERO,
        Placement::Insert { at } => at,
        Placement::Drop { start, .. } => start,
    };
    if !(0..=MAX_IPC_FLICKS).contains(&time.flicks()) {
        return Err(IpcError::InvalidEdit(format!(
            "time {} is outside 0..=2^53 flicks",
            time.flicks()
        )));
    }
    let command = AddClips::new(sources, placement);
    let ids = command.clip_ids().to_vec();
    state.edit(&app, |doc| doc.execute(Box::new(command)).map(Some))?;
    Ok(ids)
}

/// `time` if it is exact over IPC (within ±2^53 flicks, ADR-0003 §3), else `invalidEdit`.
fn ipc_time(time: Time) -> Result<Time, IpcError> {
    if time.flicks().unsigned_abs() > MAX_IPC_FLICKS.unsigned_abs() {
        return Err(IpcError::InvalidEdit(format!(
            "time {} is outside ±2^53 flicks",
            time.flicks()
        )));
    }
    Ok(time)
}

/// Moves clips by dragging, as an ADR-0006 gesture (`moveClips`): send `update` at most once per
/// animation frame, `commit` on pointer-up and `cancelGesture` on Escape, blur or lost capture.
/// The whole drag is one undo step.
///
/// `anchor` (the clip under the pointer, counted as listed) goes to the absolute `start`; the other
/// `clips` keep their offset from it, and clips linked to them come along in time on their own
/// tracks. `track` is where the anchor's clips go: allowed only when every listed clip is on the
/// anchor's track (or linked to one there), otherwise the move is time-only and `track` is
/// ignored. Pass the anchor's own track for a horizontal drag; `new.index` creates a track in the
/// section of the clip's kind. On the magnetic main track `start` picks the insertion slot (the
/// other clips whose midpoint lies before it) and the track repacks; the overlays, captions and
/// audio over moved main clips follow them. Tracks left empty are removed on `commit` only.
///
/// Errors: `overlap` (occupied spot, or a start before 0 off the magnetic main track),
/// `trackLocked`, and `invalidEdit` (unknown clip or track, a track of another kind, a time outside
/// ±2^53 flicks). A failed `update` keeps the last valid state; after a failed `commit` the
/// gesture is still open, so cancel it.
#[tauri::command]
#[specta::specta]
pub fn move_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
    anchor: ClipId,
    start: Time,
    track: TrackTarget,
    phase: EditPhase,
) -> Result<(), IpcError> {
    let command = MoveClips::new(
        clips,
        anchor,
        ipc_time(start)?,
        track,
        phase == EditPhase::Commit,
    );
    state.edit(&app, |doc| doc.edit(phase, Box::new(command)).map(Some))?;
    Ok(())
}

/// Drags one edge of a clip to the absolute `time`, as an ADR-0006 gesture (`trimClip`, same
/// lifecycle as `moveClips`). The edge is clamped, never rejected: to the media's source range
/// (`ClipView.sourceDuration`; none for images and colour cards), to at least one frame, to the
/// neighbouring clips (not on the magnetic main track) and to the same limits of linked clips whose
/// edge was at the same time, which trim with it. A start trim keeps the content in place
/// (`sourceIn` moves with the edge); on the magnetic main track the clip keeps its start instead,
/// its content shifts and the later clips ripple, with the clips over them following.
///
/// Errors: `trackLocked`, and `invalidEdit` (unknown clip, a time outside ±2^53 flicks).
#[tauri::command]
#[specta::specta]
pub fn trim_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    clip: ClipId,
    edge: Edge,
    time: Time,
    phase: EditPhase,
) -> Result<(), IpcError> {
    let command = TrimClip::new(clip, edge, ipc_time(time)?, phase == EditPhase::Commit);
    state.edit(&app, |doc| doc.edit(phase, Box::new(command)).map(Some))?;
    Ok(())
}

/// `time` if it is a timeline position that crosses IPC exactly (`0..=2^53` flicks), else
/// `invalidEdit`.
fn ipc_position(time: Time) -> Result<Time, IpcError> {
    if !(0..=MAX_IPC_FLICKS).contains(&time.flicks()) {
        return Err(IpcError::InvalidEdit(format!(
            "time {} is outside 0..=2^53 flicks",
            time.flicks()
        )));
    }
    Ok(time)
}

/// Splits clips at `at` (Ctrl+B, `splitClips`) and returns the ids of the right halves (select
/// them), in track order. With `clips` empty, every clip under `at` on an unlocked track splits;
/// otherwise the listed clips and their linked members do. A clip splits only when both halves
/// are at least one project frame long; the right halves of a link group share a new link.
///
/// Errors: `trackLocked` (a listed clip or linked member on a locked track), `invalidEdit` (no clip
/// to split, unknown clip, a time outside `0..=2^53` flicks).
#[tauri::command]
#[specta::specta]
pub fn split_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
    at: Time,
) -> Result<Vec<ClipId>, IpcError> {
    let command = SplitClips::new(clips, ipc_position(at)?);
    let added = state.edit(&app, |doc| {
        let before = doc.snapshot();
        doc.execute(Box::new(command))?;
        Ok(Some(app_core::timeline::added_clips(
            &before,
            doc.project(),
        )))
    })?;
    Ok(added.unwrap_or_default())
}

/// Deletes clips and their linked members (Delete: `ripple: false`, `deleteClips`; Shift+Delete:
/// `ripple: true`, `rippleDeleteClips`). On the magnetic main track Delete always closes the gap.
/// When main-track clips close up, the overlays, captions and audio over later clips follow, and
/// the ones entirely inside a deleted main clip are deleted with it. With `ripple` and no
/// main-track clip selected, each track that lost clips closes its gaps (linked members move too).
/// A plain Delete elsewhere leaves a gap. Tracks the delete emptied are removed.
///
/// Errors: `trackLocked`, `overlap` (a ripple would move a linked clip into another), and
/// `invalidEdit` (no clips, unknown clip).
#[tauri::command]
#[specta::specta]
pub fn delete_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
    ripple: bool,
) -> Result<(), IpcError> {
    let command = DeleteClips::new(clips, ripple);
    state.edit(&app, |doc| doc.execute(Box::new(command)).map(Some))?;
    Ok(())
}

/// Closes the gap that contains `at` on `track` (`deleteGap`): the later clips on that track and
/// their linked members move left by its length; on the main track the clips over them follow.
/// A gap is the free span before the first clip or between two clips, never after the last.
///
/// Errors: `trackLocked`, `overlap` (a linked clip would collide), `invalidEdit` (no gap at `at`,
/// unknown track, a time outside `0..=2^53` flicks).
#[tauri::command]
#[specta::specta]
pub fn delete_gap(
    app: AppHandle,
    state: State<'_, AppState>,
    track: TrackId,
    at: Time,
) -> Result<(), IpcError> {
    let command = DeleteGap::new(track, ipc_position(at)?);
    state.edit(&app, |doc| doc.execute(Box::new(command)).map(Some))?;
    Ok(())
}

/// Deletes the part of clips left (Q, `side: left`) or right (W, `side: right`) of `at`
/// (`trimToPlayhead`). With `clips` empty, the main-track clip under `at` is trimmed. Clips that
/// `at` does not lie inside are skipped. Same clamping, linked members, ripple and following as
/// `trimClip`: on the magnetic main track a left trim keeps the clip's start, so the content that
/// was at `at` is now at the clip's start (move the playhead there).
///
/// Errors: `trackLocked`, `invalidEdit` (nothing to trim, unknown clip, a time outside
/// `0..=2^53` flicks).
#[tauri::command]
#[specta::specta]
pub fn trim_to_playhead(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
    at: Time,
    side: Side,
) -> Result<(), IpcError> {
    let command = TrimToPlayhead::new(clips, ipc_position(at)?, side);
    state.edit(&app, |doc| doc.execute(Box::new(command)).map(Some))?;
    Ok(())
}

/// Turns the magnetic main track on or off (`setMagnetic`). Turning it on closes every gap on the
/// main track, and the clips over moved main clips follow, in one undo step. Setting the value it
/// already has does nothing (no history entry, no event).
///
/// Errors: `trackLocked` (the main track is locked and would move, or a clip linked to a moved
/// main clip is on a locked track).
#[tauri::command]
#[specta::specta]
pub fn set_magnetic(app: AppHandle, state: State<'_, AppState>, on: bool) -> Result<(), IpcError> {
    state.edit(&app, |doc| {
        if doc.project().magnetic == on {
            return Ok(None);
        }
        doc.execute(Box::new(SetMagnetic::new(on))).map(Some)
    })?;
    Ok(())
}

/// Copies clips and their linked members to the app's clip clipboard (Ctrl+C). Not the system
/// clipboard; cleared when a project is created, opened or restored. No undo step, no event.
///
/// Errors: `invalidEdit` (no clips, unknown clip).
#[tauri::command]
#[specta::specta]
pub fn copy_clips(state: State<'_, AppState>, clips: Vec<ClipId>) -> Result<(), IpcError> {
    state.copy_clips(&clips)
}

/// Copies clips to the clip clipboard and deletes them, as one undo step (Ctrl+X, `cutClips`).
/// The delete is a plain Delete (it closes the gap on the magnetic main track). The clipboard
/// changes only if the delete succeeds. Errors as `deleteClips`.
#[tauri::command]
#[specta::specta]
pub fn cut_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
) -> Result<(), IpcError> {
    state.cut_clips(&app, clips)
}

/// Pastes the clip clipboard at `at` (Ctrl+V, `pasteClips`) and returns the new clips' ids
/// (select them), in clipboard order: track order, then time. Main-track clips go into the main
/// track (on the magnetic one at the clip boundary nearest `at`, with a ripple; the other pasted
/// clips keep their offsets to them). Other clips go to their original track at `at` plus their
/// offset when it is free and unlocked, else onto a new track above (video, text) or below
/// (audio) the main track. Pasted link groups get new links.
///
/// Errors: `invalidEdit` (the clipboard is empty, copied media was removed since, a time outside
/// `0..=2^53` flicks), `trackLocked`.
#[tauri::command]
#[specta::specta]
pub fn paste_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    at: Time,
) -> Result<Vec<ClipId>, IpcError> {
    state.paste_clips(&app, ipc_position(at)?)
}

/// Duplicates clips and their linked members right after the latest end among them (Ctrl+D,
/// `duplicateClips`), like a paste there, without touching the clipboard. Returns the new clips'
/// ids (select them).
///
/// Errors: `invalidEdit` (no clips, unknown clip), `trackLocked`.
#[tauri::command]
#[specta::specta]
pub fn duplicate_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    clips: Vec<ClipId>,
) -> Result<Vec<ClipId>, IpcError> {
    let ids = state.edit(&app, |doc| {
        let command = PasteClips::duplicate(doc.project(), &clips)?;
        let ids = command.clip_ids().to_vec();
        doc.execute(Box::new(command))?;
        Ok(Some(ids))
    })?;
    Ok(ids.unwrap_or_default())
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

/// App version and the GPU capability report. Call once at startup; afterwards listen to
/// `appInfoChanged`.
#[tauri::command]
#[specta::specta]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        gpu: state.preview.as_ref().map(|p| (&*p.gpu_info()).into()),
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

/// Moves the preview to `time` and redraws it: `scrub` while the playhead is dragged, `exact` for
/// a click and at the end of a drag. `time` is clamped to `0..=` the timeline duration. Returns the
/// seek generation, which increases with every seek (playback ticks of an older generation are
/// stale, task 0330). Fails with `invalidEdit` for a time outside ±2^53 flicks and with
/// `unavailable` when there is no preview engine. Never blocks.
#[tauri::command]
#[specta::specta]
pub fn seek(state: State<'_, AppState>, time: Time, mode: SeekMode) -> Result<u32, IpcError> {
    state.seek(time, mode.into())
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
        let loaded = project_file::read_project_file(&path).map(|mut loaded| {
            // ADR-0002 §7: media next to the project (relative path) wins over the absolute path.
            media_paths::resolve_paths(&mut loaded.project, &path);
            loaded
        });
        (path, loaded)
    })
    .await?;
    let loaded = loaded?;
    let entry = RecentEntry::new(&loaded.project, &path);
    state.replace_project(&app, loaded.project, Some(path), loaded.source_version)?;
    // Missing or changed media files, in the background.
    state.check_media();
    // Previews missing from the cache (another machine, an evicted or cleared cache).
    state.prefetch_previews(None);
    state.record_recent(entry).await;
    Ok(())
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
        // Media paths relative to this file (ADR-0002 §7); the open document is not changed.
        let json = serialization::to_json(&media_paths::with_relative_paths(&project, Some(&path)));
        let written = project_file::write_project_file(&path, &json, backup);
        (path, written)
    })
    .await?;
    written.map_err(|err| IpcError::io(&err))?;
    let entry = RecentEntry::new(&snapshot.project, &path);
    state.mark_saved(app, &snapshot, path)?;
    state.record_recent(entry).await;
    Ok(())
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
    let restored = tauri::async_runtime::spawn_blocking(move || {
        store.load(id).map(|mut restored| {
            if let Some(file) = &restored.file_path {
                media_paths::resolve_paths(&mut restored.project, file);
            }
            restored
        })
    })
    .await??;
    state.replace_restored(&app, restored, id)?;
    state.check_media();
    // A restored project may hold media whose previews are not cached yet.
    state.prefetch_previews(None);
    Ok(())
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

/// Starts importing `paths` (files or folders) and returns the job id at once. Progress comes as
/// `jobProgress`, per-file outcomes as `mediaJobReport`, the end as `jobFinished`; all files that
/// import successfully are added as one undo step (`importMedia`). Fails with `invalidEdit` for an
/// empty list.
#[tauri::command]
#[specta::specta]
pub fn import_media(state: State<'_, AppState>, paths: Vec<String>) -> Result<JobId, IpcError> {
    if paths.is_empty() {
        return Err(IpcError::InvalidEdit("no paths to import".into()));
    }
    Ok(import::start(
        &state.jobs,
        paths.into_iter().map(PathBuf::from).collect(),
    ))
}

/// Opens the native multi-select file dialog, filtered to `MEDIA_EXTENSIONS`, and starts an
/// import of the chosen files. `null` if the user cancelled the dialog. `filterLabel` is the
/// translated name of the file-type filter (Rust sends no user-facing text). The dialog runs
/// without holding the document.
#[tauri::command]
#[specta::specta]
pub async fn import_media_dialog(
    app: AppHandle,
    filter_label: String,
) -> Result<Option<JobId>, IpcError> {
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    let mut dialog = app
        .dialog()
        .file()
        .add_filter(filter_label, engine::media::MEDIA_EXTENSIONS);
    if let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_files(move |files| {
        tx.try_send(files).ok();
    });
    let Some(files) = rx.recv().await.flatten() else {
        return Ok(None);
    };
    let paths: Vec<PathBuf> = files
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let state = app.state::<AppState>();
    Ok(Some(import::start(&state.jobs, paths)))
}

/// Asks a job to stop; its `jobFinished` result is then an `Err` of kind `cancelled`. Unknown or
/// finished ids are ignored.
#[tauri::command]
#[specta::specta]
pub fn cancel_job(state: State<'_, AppState>, id: JobId) -> Result<(), IpcError> {
    state.jobs.cancel(id);
    Ok(())
}

/// Removes media items from the project as one undo step (`removeMedia`), together with every
/// clip that uses them (the UI warns first when `MediaView.usedBy` is not 0). Main-track gaps
/// close and the clips over them follow; a canvas background showing a removed image goes back to
/// black. Fails with `invalidEdit` for an empty list, an id that is not in the project, or a clip
/// of the media on a locked track. Cached previews stay on disk, so undo shows them again at once.
#[tauri::command]
#[specta::specta]
pub fn remove_media(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<MediaId>,
) -> Result<(), IpcError> {
    if ids.is_empty() {
        return Err(IpcError::InvalidEdit("no media to remove".into()));
    }
    let mut commands: Vec<Box<dyn Command>> = ids
        .iter()
        .map(|&id| Box::new(RemoveClipsOfMedia::new(id)) as Box<dyn Command>)
        .collect();
    commands.push(Box::new(RemoveMedia::new(ids)));
    let batch = Batch::new(HistoryLabel::RemoveMedia, commands);
    state.edit(&app, |doc| doc.execute(Box::new(batch)).map(Some))?;
    Ok(())
}

/// Points media `id` at the file at `path` (absolute), e.g. after it was moved or renamed, and
/// returns the job id at once (stage `probing`). The file is fingerprinted and probed; the outcome
/// comes as `mediaJobReport` with one entry, then `jobFinished`:
/// - `relinked`: done, one undo step (`relinkMedia`);
/// - `fingerprintMismatch`: the file has other content and `allowMismatch` was `false`; nothing
///   changed. Ask the user and call again with `allowMismatch = true` to use it anyway (also the
///   way to accept a `changed` file as it is now);
/// - `failed` with `kindMismatch` (another kind, e.g. audio for a video) or an import error kind.
///
/// Fails with `invalidEdit` for an id that is not in the project.
#[tauri::command]
#[specta::specta]
pub fn relink_media(
    state: State<'_, AppState>,
    id: MediaId,
    path: String,
    allow_mismatch: bool,
) -> Result<JobId, IpcError> {
    ensure_media(&state, id)?;
    Ok(relink::start_one(
        &state.jobs,
        id,
        PathBuf::from(path),
        allow_mismatch,
    ))
}

/// Searches `folder` (absolute, recursively) for every media item whose file is missing and
/// relinks all it finds as one undo step (`relinkMedia`). Matches by content only (same file
/// name tried first); a file with the right name but other content is never taken. Returns the job
/// id at once (stage `searchingFolder`); `mediaJobReport` lists each missing item in project
/// order as `relinked` (with its new path) or `notFound` (with its old path), then `jobFinished`,
/// which fails with `fileNotFound` or `io` when the folder cannot be read.
#[tauri::command]
#[specta::specta]
pub fn relink_media_folder(state: State<'_, AppState>, folder: String) -> Result<JobId, IpcError> {
    if folder.is_empty() {
        return Err(IpcError::InvalidEdit("no folder to search".into()));
    }
    Ok(relink::start_folder(&state.jobs, PathBuf::from(folder)))
}

fn ensure_media(state: &AppState, id: MediaId) -> Result<(), IpcError> {
    if state.snapshot().media.iter().any(|m| m.id == id) {
        Ok(())
    } else {
        Err(IpcError::InvalidEdit(format!(
            "media {id} is not in the project"
        )))
    }
}

/// Opens the file manager (Explorer) at the media file's folder with the file selected. By id, so
/// the webview never hands a path to the shell. Fails with `invalidEdit` for an unknown id and
/// `fileNotFound` when the file is gone.
#[tauri::command]
#[specta::specta]
pub async fn reveal_media(state: State<'_, AppState>, id: MediaId) -> Result<(), IpcError> {
    let project = state.snapshot();
    let asset = project
        .media
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| IpcError::InvalidEdit(format!("media {id} is not in the project")))?;
    let path = PathBuf::from(&asset.path.absolute);
    tauri::async_runtime::spawn_blocking(move || {
        if !path.exists() {
            return Err(IpcError::FileNotFound(format!(
                "{} does not exist",
                path.display()
            )));
        }
        tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|err| {
            IpcError::Unavailable(format!("cannot reveal {}: {err}", path.display()))
        })
    })
    .await?
}

/// The project path passed on the command line at startup (a double-clicked project file), once;
/// later calls return `None`. Call after the first project state, before the recovery check.
#[tauri::command]
#[specta::specta]
pub fn take_launch_path(state: State<'_, AppState>) -> Option<String> {
    state
        .take_launch_path()
        .map(|path| path.to_string_lossy().into_owned())
}

/// The preferences in effect (read at startup).
#[tauri::command]
#[specta::specta]
pub fn get_preferences(state: State<'_, AppState>) -> Preferences {
    state.preferences().get()
}

/// Validates and replaces all preferences, writes the file, applies them (autosave interval, cache
/// folder) and returns what is now in effect. Fails with `invalidEdit` (the message names the
/// field) when a value is out of range or `schemaVersion` is not current, and with `io` when the
/// file or the cache folder cannot be written; then nothing changes. Not an undo step.
#[tauri::command]
#[specta::specta]
pub async fn set_preferences(
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<Preferences, IpcError> {
    if preferences.schema_version != CURRENT_PREFERENCES_VERSION {
        return Err(invalid(PreferencesError::WrongVersion(
            preferences.schema_version,
        )));
    }
    preferences.validate().map_err(invalid)?;

    let store = state.preferences();
    let _writing = store.write_lock().lock().await;
    let root = preferences.cache_root(store.app_cache_dir());
    let (path, backup) = (store.path().to_owned(), store.backup_of_version());
    let (to_write, created) = (preferences.clone(), root.clone());
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&created)?;
        preferences::write(&path, &to_write, backup)
    })
    .await?
    .map_err(|err| IpcError::io(&err))?;

    let previous = store.replace(preferences.clone(), root.clone());
    if previous.cache_root(store.app_cache_dir()) != root {
        // Previews move to the new folder at once; the old one is left alone (it regenerates).
        state.previews.set_cache_root(&root);
    }
    if previous.autosave_interval != preferences.autosave_interval {
        state.set_autosave_interval(preferences.autosave_duration());
    }
    Ok(preferences)
}

fn invalid(err: PreferencesError) -> IpcError {
    IpcError::InvalidEdit(err.to_string())
}

/// The effective cache folder and what it holds.
#[tauri::command]
#[specta::specta]
pub async fn get_cache_info(state: State<'_, AppState>) -> Result<CacheInfo, IpcError> {
    let root = state.preferences().cache_root();
    let stats = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        move || preferences::cache_info(&root)
    })
    .await?
    .map_err(|err| IpcError::io(&err))?;
    Ok(CacheInfo::new(&root, stats))
}

/// Deletes everything inside the cache folder (never the folder itself or anything outside it).
/// Returns the info after clearing.
#[tauri::command]
#[specta::specta]
pub async fn clear_cache(state: State<'_, AppState>) -> Result<CacheInfo, IpcError> {
    let store = state.preferences();
    let _changing = store.write_lock().lock().await;
    let root = store.cache_root();
    let stats = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        move || {
            preferences::clear_cache(&root)?;
            preferences::cache_info(&root)
        }
    })
    .await?
    .map_err(|err| IpcError::io(&err))?;
    // Forget remembered failures and session state of the deleted previews.
    state.previews.set_cache_root(&root);
    Ok(CacheInfo::new(&root, stats))
}

/// Recent projects for the home screen, newest first, each with its file's modification time
/// (`modifiedMs: null` = missing). Never removes an entry; a missing or broken list is empty.
#[tauri::command]
#[specta::specta]
pub async fn list_recent_projects(
    state: State<'_, AppState>,
) -> Result<Vec<RecentProject>, IpcError> {
    let listed = with_recent(&state, |store, _| Ok(store.list())).await?;
    Ok(listed.into_iter().map(RecentProject::from).collect())
}

/// Removes `path` from the recent list (the file is not touched). An unknown path is fine.
#[tauri::command]
#[specta::specta]
pub async fn remove_recent_project(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), IpcError> {
    with_recent(&state, move |store, _| {
        store.remove(&path).map_err(|err| IpcError::io(&err))
    })
    .await
}

/// Renames the recent project's file to `<name>.cutframe` in its folder (characters Windows
/// forbids become `_`) and sets the project name inside it to `name` (trimmed). Fails with
/// `invalidEdit` when the name is empty, a different file with that name exists, or `path` is not
/// a recent project or is the open one; with `fileNotFound` / `malformedProject` /
/// `projectTooNew` / `invalidProject` when the file cannot be opened; with `io` when the rename or
/// write fails (then nothing changed). Not an undo step.
#[tauri::command]
#[specta::specta]
pub async fn rename_project_file(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> Result<(), IpcError> {
    with_recent(&state, move |store, open| store.rename(&path, &name, open)).await
}

/// Copies the recent project to `<name>.cutframe` in its folder (`<name> (2).cutframe`, ... when
/// taken) as a new project called `name`, and puts the copy first in the recent list. The UI
/// passes the translated copy name. Errors as for `renameProjectFile`. Not an undo step.
#[tauri::command]
#[specta::specta]
pub async fn duplicate_project_file(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> Result<(), IpcError> {
    with_recent(&state, move |store, open| {
        store.duplicate(&path, &name, open).map(drop)
    })
    .await
}

/// Moves the recent project's file to the Recycle Bin (its `.v<n>.bak` backups stay) and removes
/// it from the list. Ask for confirmation first. Fails with `invalidEdit` when `path` is not a
/// recent project or is the open one, `fileNotFound` when the file is gone, and `io` when the
/// Recycle Bin refuses it (then the entry stays).
#[tauri::command]
#[specta::specta]
pub async fn trash_project_file(state: State<'_, AppState>, path: String) -> Result<(), IpcError> {
    with_recent(&state, move |store, open| {
        store.trash(&path, open, recent::move_to_trash)
    })
    .await
}

/// Opens Explorer at the recent project's folder with its file selected. Fails with
/// `invalidEdit` when `path` is not a recent project or is the open one, `fileNotFound` when the
/// file is gone and `unavailable` when the file manager cannot be started.
#[tauri::command]
#[specta::specta]
pub async fn reveal_project_file(state: State<'_, AppState>, path: String) -> Result<(), IpcError> {
    let file = with_recent(&state, move |store, open| store.existing_file(&path, open)).await?;
    tauri::async_runtime::spawn_blocking(move || {
        tauri_plugin_opener::reveal_item_in_dir(&file).map_err(|err| {
            IpcError::Unavailable(format!("cannot reveal {}: {err}", file.display()))
        })
    })
    .await?
}

/// Runs `action` on the recent list off the main thread, holding the recent lock, with the open
/// project's file for the guards.
async fn with_recent<T: Send + 'static>(
    state: &AppState,
    action: impl FnOnce(&RecentStore, Option<&Path>) -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    let _changing = state.recent_lock().lock().await;
    let store = state.recent().clone();
    let open = state.open_file();
    tauri::async_runtime::spawn_blocking(move || action(&store, open.as_deref())).await?
}
