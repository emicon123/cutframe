//! Data shapes sent to the UI. These are views of the model, not the model itself: they contain
//! what the UI needs to draw and nothing that would tie the UI to internal structures.

use std::path::Path;

use app_core::command::HistoryLabel;
use app_core::{ClipId, Document, ProjectId, ProjectSettings, Time, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    /// Increases on every change and is never reset during the app session, not even when another
    /// project is opened or created. Keep the payload with the highest revision.
    pub revision: u64,
    pub project: ProjectView,
    pub history: HistoryState,
    /// Absolute path of the open project file; `None` for a new project that was never saved.
    pub file_path: Option<String>,
    /// Unsaved changes: the history moved since the last save or open, or a gesture is open.
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    /// Key of the entry `undo` would revert (e.g. `addTrack`). The UI translates it.
    pub undo_label: Option<HistoryLabel>,
    /// Key of the entry `redo` would re-apply. The UI translates it.
    pub redo_label: Option<HistoryLabel>,
    /// Every history entry, oldest first: the applied ones, then the undone ones `redo` can
    /// re-apply. At most `DEFAULT_HISTORY_LIMIT` (500); older entries are dropped. An open
    /// gesture is not an entry.
    pub entries: Vec<HistoryEntryView>,
    /// How many of `entries` are applied: 0 = the oldest state in the history, `entries.length`
    /// = nothing to redo. Pass a value in `0..=entries.length` to `jumpToHistory`.
    pub current: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryView {
    /// Key of the edit (e.g. `addTrack`). The UI translates it.
    pub label: HistoryLabel,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: ProjectId,
    pub name: String,
    pub settings: ProjectSettings,
    /// End of the last clip, in flicks.
    pub duration: Time,
    /// Top to bottom as shown in the timeline.
    pub tracks: Vec<TrackView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub muted: bool,
    pub hidden: bool,
    pub clips: Vec<ClipView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: ClipId,
    pub name: String,
    pub start: Time,
    pub duration: Time,
}

impl ProjectState {
    /// The view of `doc` at the session-wide `revision` (owned by `AppState`), opened from or last
    /// saved to `file_path`.
    pub fn new(doc: &Document, revision: u64, file_path: Option<&Path>, dirty: bool) -> Self {
        let project = doc.project();
        Self {
            revision,
            project: ProjectView {
                id: project.id,
                name: project.name.clone(),
                settings: project.settings,
                duration: project.duration(),
                tracks: project
                    .tracks
                    .iter()
                    .map(|t| TrackView {
                        id: t.id,
                        kind: t.kind,
                        name: t.name.clone(),
                        muted: t.muted,
                        hidden: t.hidden,
                        clips: t
                            .clips
                            .iter()
                            .map(|c| ClipView {
                                id: c.id,
                                name: c.name.clone(),
                                start: c.start,
                                duration: c.duration,
                            })
                            .collect(),
                    })
                    .collect(),
            },
            history: HistoryState {
                can_undo: doc.can_undo(),
                can_redo: doc.can_redo(),
                undo_label: doc.undo_label(),
                redo_label: doc.redo_label(),
                entries: doc
                    .history_labels()
                    .map(|label| HistoryEntryView { label })
                    .collect(),
                // Capped by `DEFAULT_HISTORY_LIMIT`, far below `u32::MAX`.
                current: u32::try_from(doc.history_cursor()).unwrap_or(u32::MAX),
            },
            file_path: file_path.map(|p| p.to_string_lossy().into_owned()),
            dirty,
        }
    }
}

/// An autosave left by an instance that did not exit cleanly, offered for restore at startup.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    /// Session id of the crashed instance; pass it to `restoreRecovery` / `discardRecovery`.
    pub id: Uuid,
    /// The project's name (model data, not translated).
    pub project_name: String,
    /// The project's file at the time of the autosave; `None` = never saved.
    pub file_path: Option<String>,
    /// When the autosave was taken, in Unix milliseconds. The UI formats it.
    pub saved_at_ms: i64,
}

impl From<crate::recovery::Entry> for RecoveryEntry {
    fn from(entry: crate::recovery::Entry) -> Self {
        Self {
            id: entry.id,
            project_name: entry.project_name,
            file_path: entry.file_path.map(|p| p.to_string_lossy().into_owned()),
            saved_at_ms: entry.saved_at_ms,
        }
    }
}

/// How the preview reaches the screen on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PreviewMode {
    /// A native GPU window is placed over the preview area (Windows). The DOM must not draw there.
    Native,
    /// The UI pulls frames from `media://localhost/preview/frame` and draws them (Linux dev mode).
    Frames,
    /// No GPU available; the UI shows a placeholder.
    Unavailable,
}

/// The preview area in physical pixels, relative to the webview's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfoView {
    pub adapter: String,
    pub backend: String,
    pub software: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub gpu: Option<GpuInfoView>,
    pub preview_mode: PreviewMode,
}
