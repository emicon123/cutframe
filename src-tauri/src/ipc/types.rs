//! Data shapes sent to the UI. These are views of the model, not the model itself: they contain
//! what the UI needs to draw and nothing that would tie the UI to internal structures.

use std::path::Path;

use app_core::command::HistoryLabel;
use app_core::media::{FilmstripLayout, HdrTransfer, MediaAsset};
use app_core::{
    ClipId, Document, MediaId, MediaKind, ProjectId, ProjectSettings, Rational, Time, TrackId,
    TrackKind,
};
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
    /// Imported media in import order. The media panel sorts and filters its own view of it.
    pub media: Vec<MediaView>,
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
                media: project.media.iter().map(MediaView::new).collect(),
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

/// One imported media item as the media panel shows it. No fingerprint, size or mtime crosses
/// IPC (design §4).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaView {
    pub id: MediaId,
    pub kind: MediaKind,
    pub file_name: String,
    /// Absolute path, for the list view and tooltips.
    pub path: String,
    /// `None` for images.
    pub duration: Option<Time>,
    /// The upright picture size on screen (after rotation and pixel aspect); `None` for audio.
    pub display_width: Option<u32>,
    pub display_height: Option<u32>,
    /// Source frame rate, informational; `None` for audio and images.
    pub frame_rate: Option<Rational>,
    pub video_codec: Option<String>,
    /// HDR badge; `dolbyVisionOnly` gets a warning (its colours will be wrong).
    pub hdr: Option<HdrTransfer>,
    /// The default audio stream; `None` without audio.
    pub audio: Option<AudioSummary>,
    /// Always `online` until missing-media detection lands (task 0240).
    pub status: MediaStatus,
    /// Clips that reference this media; 0 until clips reference media (design 0250).
    pub used_by: u32,
    /// Append as `?v=` to every `media://` URL of this item, so a relinked or changed file never
    /// shows stale previews.
    pub preview_version: String,
    /// The filmstrip of a video (hover-scrub tiles at `media://…/filmstrip/<id>?tile=<n>`);
    /// `None` for audio and images.
    pub filmstrip: Option<FilmstripLayout>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioSummary {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Number of audio streams in the file (OBS-style recordings have several).
    pub stream_count: u32,
}

/// Whether the media file is reachable. Session state, not saved (design §6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MediaStatus {
    Online,
    /// The file is missing (task 0240).
    Offline,
    /// The file at the path has other content than when it was imported (task 0240).
    Changed,
}

impl MediaView {
    pub fn new(asset: &MediaAsset) -> Self {
        let video = asset.info.video.as_ref();
        let display = video.map(|v| v.display_size());
        Self {
            id: asset.id,
            kind: asset.kind,
            file_name: asset.file_name().to_owned(),
            path: asset.path.absolute.clone(),
            duration: asset.info.duration,
            display_width: display.map(|(w, _)| w),
            display_height: display.map(|(_, h)| h),
            frame_rate: video.and_then(|v| v.frame_rate),
            video_codec: video.map(|v| v.codec.clone()),
            hdr: video.and_then(|v| v.hdr).map(|h| h.transfer),
            audio: asset.info.audio.first().map(|a| AudioSummary {
                codec: a.codec.clone(),
                sample_rate: a.sample_rate,
                channels: a.channels,
                stream_count: asset.info.audio.len() as u32,
            }),
            status: MediaStatus::Online,
            used_by: 0,
            preview_version: crate::previews::preview_version(asset),
            filmstrip: FilmstripLayout::of(asset),
        }
    }
}

/// Identifies a background job (import, later previews, export). UUID v7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

/// What a job is doing; a stable key the UI translates (ADR-0003 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    /// Reading and probing media files (import).
    Probing,
    /// Making posters, waveforms and filmstrips in the background (after import and open).
    GeneratingPreviews,
}

/// The result of one file of a media job, in the order the files were given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaFileResult {
    /// Absolute path of the file (or of the folder, for a folder-level failure).
    pub path: String,
    pub outcome: MediaFileOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MediaFileOutcome {
    Added {
        media: MediaId,
    },
    /// The same content (fingerprint) is already in the project, or earlier in the same import.
    AlreadyImported {
        media: MediaId,
    },
    Failed {
        error: MediaErrorKind,
        /// English, for the log only.
        detail: String,
    },
}

/// Why a file could not be imported; a stable key the UI translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MediaErrorKind {
    FileNotFound,
    AccessDenied,
    /// The path cannot be used (e.g. not valid UTF-8, not a file).
    UnsupportedPath,
    /// No demuxer recognises the file, or it has no video or audio stream.
    UnsupportedFormat,
    /// A stream exists but this build cannot decode it.
    UnsupportedCodec,
    /// The file is truncated or corrupt, or its duration is unknown.
    Damaged,
    /// A dropped folder holds more media files than one import takes (`MAX_IMPORT_FILES`);
    /// nothing from it was imported.
    TooManyFiles,
}

impl From<engine::media::MediaErrorKind> for MediaErrorKind {
    fn from(kind: engine::media::MediaErrorKind) -> Self {
        use engine::media::MediaErrorKind as E;
        match kind {
            E::FileNotFound => Self::FileNotFound,
            E::AccessDenied => Self::AccessDenied,
            E::UnsupportedPath => Self::UnsupportedPath,
            E::UnsupportedFormat => Self::UnsupportedFormat,
            E::UnsupportedCodec => Self::UnsupportedCodec,
            // A cancelled file never gets a result; the job reports `cancelled` instead.
            E::Damaged | E::Cancelled => Self::Damaged,
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
