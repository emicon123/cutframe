//! Data shapes sent to the UI. These are views of the model, not the model itself: they contain
//! what the UI needs to draw and nothing that would tie the UI to internal structures.

use app_core::command::HistoryLabel;
use app_core::{ClipId, Document, ProjectId, ProjectSettings, Time, TrackId, TrackKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    /// Increases on every change and is never reset during the app session, not even when another
    /// project is opened or created. Keep the payload with the highest revision.
    pub revision: u64,
    pub project: ProjectView,
    pub history: HistoryState,
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
    /// The view of `doc` at the session-wide `revision` (owned by `AppState`).
    pub fn new(doc: &Document, revision: u64) -> Self {
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
            },
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
