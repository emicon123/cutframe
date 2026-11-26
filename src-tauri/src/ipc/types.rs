//! Data shapes sent to the UI. These are views of the model, not the model itself: they contain
//! what the UI needs to draw and nothing that would tie the UI to internal structures.

use std::path::Path;

use app_core::command::HistoryLabel;
use app_core::media::{FilmstripLayout, HdrTransfer, MediaAsset};
use app_core::{
    AudioProps, CanvasBackground, Clip, ClipId, ClipSource, Document, LinkId, MediaId, MediaKind,
    Project, ProjectId, ProjectSettings, Rational, Time, Track, TrackId, TrackKind, VisualProps,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::media_status::MediaStatuses;

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
    /// What fills the canvas behind the main track.
    pub background: CanvasBackground,
    /// The magnetic main track (always a video track in `tracks`).
    pub main_track: TrackId,
    /// While on, the main track has no gaps.
    pub magnetic: bool,
    /// Top to bottom as shown in the timeline: overlay tracks (video, text), the main track, then
    /// audio tracks.
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
    pub locked: bool,
    pub clips: Vec<ClipView>,
}

impl TrackView {
    fn new(track: &Track, project: &Project) -> Self {
        Self {
            id: track.id,
            kind: track.kind,
            name: track.name.clone(),
            muted: track.muted,
            hidden: track.hidden,
            locked: track.locked,
            clips: track
                .clips
                .iter()
                .map(|c| ClipView::new(c, project))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: ClipId,
    pub name: String,
    pub start: Time,
    pub duration: Time,
    /// Media (with `sourceIn`), colour or text.
    pub source: ClipSource,
    /// Clips with the same link move, trim, split and delete together.
    pub link: Option<LinkId>,
    /// Present on clips of video and text tracks.
    pub visual: Option<VisualProps>,
    /// Present on clips that make sound.
    pub audio: Option<AudioProps>,
    /// Length of the clip's media, for trim limits; `None` for images, colour and text clips.
    pub source_duration: Option<Time>,
}

impl ClipView {
    fn new(clip: &Clip, project: &Project) -> Self {
        Self {
            id: clip.id,
            name: clip.name.clone(),
            start: clip.start,
            duration: clip.duration,
            source: clip.source.clone(),
            link: clip.link,
            visual: clip.visual.clone(),
            audio: clip.audio.clone(),
            source_duration: clip
                .source
                .media()
                .and_then(|id| project.media_asset(id))
                .and_then(|m| m.info.duration),
        }
    }
}

impl ProjectState {
    /// The view of `doc` at the session-wide `revision` (owned by `AppState`), opened from or last
    /// saved to `file_path`.
    /// `statuses` is the session's media status (design §6.6).
    pub fn new(
        doc: &Document,
        revision: u64,
        file_path: Option<&Path>,
        dirty: bool,
        statuses: &MediaStatuses,
    ) -> Self {
        let project = doc.project();
        Self {
            revision,
            project: ProjectView {
                id: project.id,
                name: project.name.clone(),
                settings: project.settings,
                duration: project.duration(),
                background: project.background.clone(),
                main_track: project.main_track,
                magnetic: project.magnetic,
                tracks: project
                    .tracks
                    .iter()
                    .map(|t| TrackView::new(t, project))
                    .collect(),
                media: {
                    let mut used_by: std::collections::HashMap<MediaId, u32> =
                        std::collections::HashMap::new();
                    for media in project.clips().filter_map(|c| c.source.media()) {
                        *used_by.entry(media).or_default() += 1;
                    }
                    project
                        .media
                        .iter()
                        .map(|m| {
                            MediaView::new(
                                m,
                                statuses.get(m),
                                used_by.get(&m.id).copied().unwrap_or(0),
                            )
                        })
                        .collect()
                },
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

/// A project on the home screen's recent list, newest first (design `home-screen.md` §5).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    /// Absolute path of the project file; pass it back to the recent-project commands.
    pub path: String,
    /// The project's name at the last open or save (model data, not translated).
    pub name: String,
    /// The project's duration at that time, flicks.
    pub duration: Time,
    /// Unix milliseconds of the last open, save or duplicate by this app.
    pub opened_at_ms: i64,
    /// The file's modification time in Unix milliseconds; `null` = the file is missing or
    /// unreachable (show it as missing, offer only `removeRecentProject`).
    pub modified_ms: Option<i64>,
    /// Content hash of the cover: `media://cover/<hash>` (404 = show a placeholder).
    pub cover: Option<String>,
}

impl From<crate::recent::Listed> for RecentProject {
    fn from(listed: crate::recent::Listed) -> Self {
        let entry = listed.entry;
        Self {
            path: entry.path,
            name: entry.name,
            duration: entry.duration,
            opened_at_ms: entry.opened_at_ms,
            modified_ms: listed.modified_ms,
            cover: entry.cover,
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
    /// Whether the file is reachable and unchanged. Session state: `online` until the background
    /// check after open says otherwise; it changes at runtime when a file disappears, and after a
    /// relink.
    pub status: MediaStatus,
    /// How many clips reference this media (the "used in timeline" badge).
    pub used_by: u32,
    /// Append as `?v=` to every `media://` URL of this item. Opaque; it changes whenever a relink
    /// changes the file's path or content, so the UI fetches the previews again (also after an
    /// item that was offline is relinked to its own content at a new place).
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
    /// The file is missing (or its drive is not reachable): relink it.
    Offline,
    /// The file at the path has other content than when it was imported or relinked; relink to
    /// the file "as it is now" (`allowMismatch`) or to the original.
    Changed,
}

impl MediaView {
    /// The view of `asset`, used by `used_by` clips.
    pub fn new(asset: &MediaAsset, status: MediaStatus, used_by: u32) -> Self {
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
            status,
            used_by,
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
    /// Checking in the background that every media file is still there and unchanged (after
    /// open and restore).
    CheckingMedia,
    /// Looking for missing media files in a folder (`relinkMediaFolder`).
    SearchingFolder,
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
    /// The media now points at this file.
    Relinked {
        media: MediaId,
    },
    /// The same content (fingerprint) is already in the project, or earlier in the same import.
    AlreadyImported {
        media: MediaId,
    },
    /// `relinkMedia` without `allowMismatch`: the file has other content than the media. Ask the
    /// user, then call `relinkMedia` again with `allowMismatch = true`. Nothing changed.
    FingerprintMismatch {
        media: MediaId,
    },
    /// `relinkMediaFolder` found no file with this missing media's content; `path` is its old
    /// path.
    NotFound {
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
    /// `relinkMedia` to a file of another kind (e.g. a video to an audio file). Nothing changed.
    KindMismatch,
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

/// How a `seek` is meant (playback.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SeekMode {
    /// A click, a step or the end of a drag: show the exact frame.
    Exact,
    /// While the playhead is dragged: show the nearest ready frame at once.
    Scrub,
}

impl From<SeekMode> for engine::transport::SeekMode {
    fn from(mode: SeekMode) -> Self {
        match mode {
            SeekMode::Exact => Self::Exact,
            SeekMode::Scrub => Self::Scrub,
        }
    }
}

/// J, K and L (`shuttle`, playback.md §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ShuttleKey {
    /// J: reverse at -1x, then faster (-2x, -4x, -8x) on repeated presses.
    Reverse,
    /// K: pause.
    Pause,
    /// L: forward at 1x, then faster (2x, 4x, 8x) on repeated presses.
    Forward,
}

impl From<ShuttleKey> for engine::playback::ShuttleKey {
    fn from(key: ShuttleKey) -> Self {
        match key {
            ShuttleKey::Reverse => Self::Reverse,
            ShuttleKey::Pause => Self::Pause,
            ShuttleKey::Forward => Self::Forward,
        }
    }
}

/// Whether the transport plays (playback.md §8). Preroll (frames and sound being prepared,
/// usually well under 250 ms) already counts as playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TransportStateView {
    Paused,
    Playing,
}

/// What the preview plays. Only the timeline so far; a media item (source preview) comes with
/// task 0340.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlaybackTarget {
    Timeline,
}

/// The GPU capability report (status bar, Preferences → Performance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfoView {
    /// Adapter name as the driver reports it (data, not translated).
    pub adapter: String,
    /// Graphics API, e.g. `Dx12` (data, not translated).
    pub backend: String,
    /// Software rasterizer (WARP): slower.
    pub software: bool,
    pub device_type: GpuDeviceType,
    /// Driver description; may be empty.
    pub driver: String,
    /// `false`: no adapter can present to the native preview window; it renders offscreen only.
    pub presents_to_window: bool,
    /// Why this is not the first-choice adapter; `None` = it is.
    pub fallback: Option<GpuFallback>,
    /// Hardware video decoders/encoders. Always `None` until media decoding exists.
    pub video_acceleration: Option<VideoAccelerationView>,
}

/// Kind of GPU. The UI translates the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum GpuDeviceType {
    Discrete,
    Integrated,
    Virtual,
    Software,
    Other,
}

/// Why the GPU in use is not the first choice. The UI translates the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum GpuFallback {
    /// `WGPU_FORCE_FALLBACK_ADAPTER` forced software rendering.
    ForcedByEnv,
    /// A preferred GPU cannot present to the preview window.
    PreferredCannotPresent,
    /// No hardware GPU: software rendering.
    NoHardwareAdapter,
    /// The GPU stopped responding at runtime and the preview was restarted on this adapter.
    DeviceLost,
}

/// Hardware video codecs, by FFmpeg name (e.g. `h264_d3d11va`). Not filled yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoAccelerationView {
    pub decoders: Vec<String>,
    pub encoders: Vec<String>,
}

impl From<&engine::render::GpuInfo> for GpuInfoView {
    fn from(info: &engine::render::GpuInfo) -> Self {
        use engine::render::{DeviceKind, FallbackReason};
        Self {
            adapter: info.adapter.clone(),
            backend: info.backend.clone(),
            software: info.software,
            device_type: match info.device_type {
                DeviceKind::Discrete => GpuDeviceType::Discrete,
                DeviceKind::Integrated => GpuDeviceType::Integrated,
                DeviceKind::Virtual => GpuDeviceType::Virtual,
                DeviceKind::Software => GpuDeviceType::Software,
                DeviceKind::Other => GpuDeviceType::Other,
            },
            driver: info.driver.clone(),
            presents_to_window: info.presents_to_window,
            fallback: info.fallback.map(|reason| match reason {
                FallbackReason::ForcedByEnv => GpuFallback::ForcedByEnv,
                FallbackReason::PreferredCannotPresent => GpuFallback::PreferredCannotPresent,
                FallbackReason::NoHardwareAdapter => GpuFallback::NoHardwareAdapter,
                FallbackReason::DeviceLost => GpuFallback::DeviceLost,
            }),
            video_acceleration: None,
        }
    }
}

/// The cache folder and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    /// The effective cache root (`<folder>/cutframe-cache`); the app only deletes inside it.
    pub path: String,
    /// Total size in bytes, as a decimal string (64-bit, ADR-0003 §3).
    pub size_bytes: String,
    pub files: u32,
}

impl CacheInfo {
    pub fn new(root: &Path, stats: crate::preferences::CacheStats) -> Self {
        Self {
            path: root.to_string_lossy().into_owned(),
            size_bytes: stats.size_bytes.to_string(),
            files: stats.files,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    /// `None` when no GPU could be initialized at startup.
    pub gpu: Option<GpuInfoView>,
    /// `unavailable` also after the preview gave up following repeated GPU losses.
    pub preview_mode: PreviewMode,
}

#[cfg(test)]
mod tests {
    use engine::render::{DeviceKind, FallbackReason, GpuInfo};

    use super::*;

    #[test]
    fn gpu_report_serializes_stable_keys() {
        let info = GpuInfo {
            adapter: "Microsoft Basic Render Driver".into(),
            backend: "Dx12".into(),
            software: true,
            device_type: DeviceKind::Software,
            driver: String::new(),
            presents_to_window: true,
            fallback: Some(FallbackReason::NoHardwareAdapter),
        };
        assert_eq!(
            serde_json::to_value(GpuInfoView::from(&info)).unwrap(),
            serde_json::json!({
                "adapter": "Microsoft Basic Render Driver",
                "backend": "Dx12",
                "software": true,
                "deviceType": "software",
                "driver": "",
                "presentsToWindow": true,
                "fallback": "noHardwareAdapter",
                "videoAcceleration": null,
            })
        );
        let keys = [
            (FallbackReason::ForcedByEnv, "forcedByEnv"),
            (
                FallbackReason::PreferredCannotPresent,
                "preferredCannotPresent",
            ),
            (FallbackReason::NoHardwareAdapter, "noHardwareAdapter"),
            (FallbackReason::DeviceLost, "deviceLost"),
        ];
        for (reason, key) in keys {
            let view = GpuInfoView::from(&GpuInfo {
                fallback: Some(reason),
                ..info.clone()
            });
            assert_eq!(serde_json::to_value(view.fallback).unwrap(), key);
        }
        let kinds = [
            (DeviceKind::Discrete, "discrete"),
            (DeviceKind::Integrated, "integrated"),
            (DeviceKind::Virtual, "virtual"),
            (DeviceKind::Software, "software"),
            (DeviceKind::Other, "other"),
        ];
        for (kind, key) in kinds {
            let view = GpuInfoView::from(&GpuInfo {
                device_type: kind,
                ..info.clone()
            });
            assert_eq!(serde_json::to_value(view.device_type).unwrap(), key);
        }
    }

    #[test]
    fn project_view_counts_media_users_and_copies_clip_sources() {
        use app_core::media::samples;

        let mut project = Project::new("View");
        project.media = vec![samples::video(1), samples::image(2)];
        let video = project.media[0].id;
        let mut clip = Clip::color("C", Time::ZERO, Time(10), app_core::Rgba8::BLACK);
        clip.source = ClipSource::Media {
            media: video,
            source_in: Time(5),
        };
        project.tracks[0].clips.push(clip);
        assert_eq!(project.validate(), Ok(()));

        let view = ProjectState::new(
            &Document::new(project.clone()),
            1,
            None,
            false,
            &crate::media_status::MediaStatuses::default(),
        )
        .project;
        assert_eq!(view.media[0].used_by, 1);
        assert_eq!(view.media[1].used_by, 0);
        assert_eq!(view.main_track, project.main_track);
        assert!(view.magnetic);
        let clip = &view.tracks[0].clips[0];
        assert_eq!(clip.source_duration, Some(Time::from_seconds(5)));
        let json = serde_json::to_value(clip).unwrap();
        assert_eq!(
            json["source"],
            serde_json::json!({ "kind": "media", "media": video, "sourceIn": 5 })
        );
        assert_eq!(json["visual"]["fit"], "fit");
        assert_eq!(
            serde_json::to_value(&view.background).unwrap(),
            serde_json::json!({ "kind": "color", "color": "#000000ff" })
        );
    }

    #[test]
    fn cache_size_is_a_string() {
        let info = CacheInfo::new(
            Path::new("/c/cutframe-cache"),
            crate::preferences::CacheStats {
                size_bytes: u64::MAX,
                files: 2,
            },
        );
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            serde_json::json!({
                "path": "/c/cutframe-cache",
                "sizeBytes": "18446744073709551615",
                "files": 2,
            })
        );
    }
}
