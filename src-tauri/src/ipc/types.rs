//! Data shapes sent to the UI. These are views of the model, not the model itself: they contain
//! what the UI needs to draw and nothing that would tie the UI to internal structures.

use std::path::Path;

use app_core::command::HistoryLabel;
use app_core::media::{FilmstripLayout, HdrTransfer, MediaAsset};
use app_core::{
    AudioProps, CanvasBackground, Clip, ClipId, ClipSource, Document, FrameRate, LinkId, MediaId,
    MediaKind, Project, ProjectId, ProjectSettings, Rational, Time, Track, TrackId, TrackKind,
    VisualProps,
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
    /// Media (with `sourceIn`, `speed` in percent and `reverse`), freeze frame (`sourceTime`),
    /// colour or text.
    pub source: ClipSource,
    /// Clips with the same link move, trim, split and delete together.
    pub link: Option<LinkId>,
    /// Present on clips of video and text tracks.
    pub visual: Option<VisualProps>,
    /// Present on clips that make sound.
    pub audio: Option<AudioProps>,
    /// Length of the clip's media, for trim limits; `None` for images, colour and text clips.
    /// Freeze frames report their video's length.
    pub source_duration: Option<Time>,
    /// End (exclusive) of the media range a video or audio clip uses: it plays `sourceIn` up to
    /// here at its speed, or from here down to `sourceIn` when reversed. Draw filmstrips and
    /// waveforms over this range. `None` for freeze, colour and text clips.
    pub source_end: Option<Time>,
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
            source_end: clip.source_end(),
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
    /// Test-encoding with each export encoder, once per session (`getExportCapabilities`).
    ProbingEncoders,
    /// Export: opening the encoder, the renderer and the output file. No progress value.
    ExportPreparing,
    /// Export: rendering and encoding frames; progress is frames done / frames.
    ExportEncoding,
    /// Export: flushing the encoders, finishing the MP4 and renaming it. No progress value.
    ExportFinalizing,
}

/// Video codec of an export (export.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl From<VideoCodec> for engine::export::plan::VideoCodec {
    fn from(codec: VideoCodec) -> Self {
        match codec {
            VideoCodec::H264 => Self::H264,
            VideoCodec::Hevc => Self::Hevc,
        }
    }
}

impl From<engine::export::plan::VideoCodec> for VideoCodec {
    fn from(codec: engine::export::plan::VideoCodec) -> Self {
        use engine::export::plan::VideoCodec as Engine;
        match codec {
            Engine::H264 => Self::H264,
            Engine::Hevc => Self::Hevc,
        }
    }
}

/// An export encoder implementation (export.md §4.4). Keys are stable; `CUTFRAME_ENCODERS`
/// uses the same ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EncoderKind {
    /// NVIDIA NVENC.
    Nvenc,
    /// Intel Quick Sync.
    Qsv,
    /// AMD AMF.
    Amf,
    /// A Media Foundation hardware encoder (the GPU driver's MFT).
    MfHardware,
    /// Microsoft's Media Foundation software H.264 encoder.
    MfSoftware,
    /// OpenH264 (software, last resort).
    OpenH264,
}

impl From<EncoderKind> for engine::export::encoders::EncoderKind {
    fn from(kind: EncoderKind) -> Self {
        match kind {
            EncoderKind::Nvenc => Self::Nvenc,
            EncoderKind::Qsv => Self::Qsv,
            EncoderKind::Amf => Self::Amf,
            EncoderKind::MfHardware => Self::MfHardware,
            EncoderKind::MfSoftware => Self::MfSoftware,
            EncoderKind::OpenH264 => Self::OpenH264,
        }
    }
}

impl From<engine::export::encoders::EncoderKind> for EncoderKind {
    fn from(kind: engine::export::encoders::EncoderKind) -> Self {
        use engine::export::encoders::EncoderKind as Engine;
        match kind {
            Engine::Nvenc => Self::Nvenc,
            Engine::Qsv => Self::Qsv,
            Engine::Amf => Self::Amf,
            Engine::MfHardware => Self::MfHardware,
            Engine::MfSoftware => Self::MfSoftware,
            Engine::OpenH264 => Self::OpenH264,
        }
    }
}

/// One export encoder candidate and whether this machine can use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EncoderStatus {
    pub codec: VideoCodec,
    pub encoder: EncoderKind,
    /// Runs on the GPU (NVENC, Quick Sync, AMF, Media Foundation hardware).
    pub hardware: bool,
    /// The session's test encode worked (`false` also when `CUTFRAME_ENCODERS` left it out).
    pub available: bool,
}

/// The export encoders of this machine (`getExportCapabilities`, `exportCapabilitiesChanged`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCapabilities {
    /// `false` until the session's probe finished; `encoders` is empty until then.
    pub probed: bool,
    /// Every candidate in priority order, H.264 first.
    pub encoders: Vec<EncoderStatus>,
}

impl ExportCapabilities {
    /// Before the probe finished.
    pub const PENDING: Self = Self {
        probed: false,
        encoders: Vec::new(),
    };
}

impl From<&engine::export::encoders::ProbeReport> for ExportCapabilities {
    fn from(report: &engine::export::encoders::ProbeReport) -> Self {
        use engine::export::encoders::ProbeOutcome;
        Self {
            probed: true,
            encoders: report
                .entries
                .iter()
                .map(|entry| EncoderStatus {
                    codec: entry.codec.into(),
                    encoder: entry.kind.into(),
                    hardware: entry.kind.is_hardware(),
                    available: matches!(entry.outcome, ProbeOutcome::Available(_)),
                })
                .collect(),
        }
    }
}

/// Output size of a video export: the canvas, or a short side of 480 ... 2160 px with the
/// canvas aspect (export.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ExportResolution {
    Project,
    P480,
    P720,
    P1080,
    P1440,
    P2160,
}

impl From<ExportResolution> for engine::export::plan::ExportResolution {
    fn from(resolution: ExportResolution) -> Self {
        match resolution {
            ExportResolution::Project => Self::Project,
            ExportResolution::P480 => Self::P480,
            ExportResolution::P720 => Self::P720,
            ExportResolution::P1080 => Self::P1080,
            ExportResolution::P1440 => Self::P1440,
            ExportResolution::P2160 => Self::P2160,
        }
    }
}

/// Quality as a factor on YouTube's upload bitrate, or an explicit bitrate (500..=200000 kbps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportQuality {
    Low,
    Recommended,
    High,
    Custom { kbps: u32 },
}

impl From<ExportQuality> for engine::export::plan::ExportQuality {
    fn from(quality: ExportQuality) -> Self {
        match quality {
            ExportQuality::Low => Self::Low,
            ExportQuality::Recommended => Self::Recommended,
            ExportQuality::High => Self::High,
            ExportQuality::Custom { kbps } => Self::Custom { kbps },
        }
    }
}

/// What the export dialog chose for a video export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoExportSettings {
    pub codec: VideoCodec,
    pub resolution: ExportResolution,
    /// `None`: the project rate (60 fps if the project is faster). Otherwise 24, 25, 30, 50, 60
    /// or the project rate.
    pub frame_rate: Option<FrameRate>,
    pub quality: ExportQuality,
    /// `None`: automatic (the best available encoder, with fallback). `Some`: only this one.
    pub encoder: Option<EncoderKind>,
}

impl From<VideoExportSettings> for engine::export::plan::VideoExportSettings {
    fn from(settings: VideoExportSettings) -> Self {
        Self {
            codec: settings.codec.into(),
            resolution: settings.resolution.into(),
            frame_rate: settings.frame_rate,
            quality: settings.quality.into(),
            encoder: settings.encoder.map(Into::into),
        }
    }
}

/// What an export produces. Task 0400 adds audio-only and still-frame targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportTarget {
    Video { settings: VideoExportSettings },
}

/// The settings of `checkExport` and `startExport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    /// Absolute path of the output file (folder + name); `.mp4` is appended when the name has no
    /// extension. User data, not user-facing text.
    pub path: String,
    pub target: ExportTarget,
}

/// A finding of `checkExport`. All but `fileExists` and `lowDiskSpace` block the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportIssue {
    /// Blocking: nothing on the timeline.
    EmptyTimeline,
    /// Blocking: not an absolute path, a reserved name or character, another extension than
    /// `.mp4`, or a folder.
    InvalidPath,
    /// Blocking: the folder does not exist.
    FolderMissing,
    /// Blocking: the path is a media file of the project.
    OutputIsSource { media: MediaId },
    /// Blocking, one per media: a file the timeline uses is missing (offer Relink).
    MediaOffline { media: MediaId },
    /// Blocking (only after the probe): no available encoder for the codec, or the chosen
    /// encoder is not available.
    CodecUnavailable,
    /// Blocking: the target file is open in another program.
    FileLocked,
    /// The target file exists: ask, then start with `overwrite`.
    FileExists,
    /// Warning: less free space than 1.2 x the estimated size.
    LowDiskSpace,
}

impl ExportIssue {
    /// The export cannot start while this is listed (`fileExists` needs `overwrite`).
    pub fn is_blocking(self) -> bool {
        !matches!(self, Self::FileExists | Self::LowDiskSpace)
    }
}

/// The plan and the issues of `checkExport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCheck {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Length of the exported video (the timeline rounded up to whole frames).
    pub duration: Time,
    pub frames: u32,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub sample_rate: u32,
    /// Estimated file size in bytes, decimal string.
    pub estimated_bytes: String,
    /// Free bytes on the target volume (decimal string); `None` if the folder does not exist or
    /// cannot be queried.
    pub free_bytes: Option<String>,
    /// The encoder automatic selection tries first (or the chosen one); `None` before the
    /// session's probe finished or if none is available.
    pub encoder: Option<EncoderKind>,
    pub issues: Vec<ExportIssue>,
    /// No blocking issue: `startExport` accepts these settings now (with `overwrite` if
    /// `fileExists` is listed).
    pub can_start: bool,
}

/// Why an export failed (export.md §5.3); a stable key the UI words its message from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ExportErrorKind {
    /// A source file is missing (`media` says which); the media panel shows it offline.
    MediaOffline,
    /// A source cannot be decoded (`media`).
    DecodeFailed,
    /// No encoder candidate opened at the export settings.
    NoEncoder,
    /// The encoder failed after the export started writing, or the chosen encoder failed.
    EncoderFailed,
    /// The output volume is full.
    DiskFull,
    /// Any other output error (access denied, folder gone, drive removed, final rename failed).
    WriteFailed,
    /// The GPU failed (device lost).
    Gpu,
    /// A bug; details are in the log.
    Internal,
}

impl From<engine::export::run::ExportErrorKind> for ExportErrorKind {
    fn from(kind: engine::export::run::ExportErrorKind) -> Self {
        use engine::export::run::ExportErrorKind as Engine;
        match kind {
            Engine::MediaOffline => Self::MediaOffline,
            Engine::DecodeFailed => Self::DecodeFailed,
            Engine::NoEncoder => Self::NoEncoder,
            Engine::EncoderFailed => Self::EncoderFailed,
            Engine::DiskFull => Self::DiskFull,
            Engine::WriteFailed => Self::WriteFailed,
            Engine::Gpu => Self::Gpu,
            Engine::Internal => Self::Internal,
        }
    }
}

/// How an export ended (`exportReport`); a cancelled export has no report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportOutcome {
    #[serde(rename_all = "camelCase")]
    Done {
        /// Absolute path of the finished file.
        path: String,
        /// File size in bytes, decimal string.
        bytes: String,
        /// The encoder that wrote the file (after any fallback).
        encoder: EncoderKind,
        elapsed_ms: u32,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        error: ExportErrorKind,
        /// The media of `mediaOffline` and `decodeFailed`.
        media: Option<MediaId>,
        /// A finished file kept under its partial name because the final rename failed.
        kept_path: Option<String>,
        /// English, for the log only.
        detail: String,
    },
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

/// What the preview plays (`setPlaybackTarget`, playback.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlaybackTarget {
    /// The timeline (the default).
    Timeline,
    /// One media item on its own (source preview): its picture fitted, its default audio
    /// stream, its own length and frame rate. Transport times are media times.
    Media { media: MediaId },
}

impl From<PlaybackTarget> for engine::playback::PlaybackTarget {
    fn from(target: PlaybackTarget) -> Self {
        match target {
            PlaybackTarget::Timeline => Self::Timeline,
            PlaybackTarget::Media { media } => Self::Media(media),
        }
    }
}

impl From<engine::playback::PlaybackTarget> for PlaybackTarget {
    fn from(target: engine::playback::PlaybackTarget) -> Self {
        match target {
            engine::playback::PlaybackTarget::Timeline => Self::Timeline,
            engine::playback::PlaybackTarget::Media(media) => Self::Media { media },
        }
    }
}

/// The preview's resolution (`setPreviewQuality`, render-graph.md §8): the composite at full,
/// half or quarter of the preview size. A UI preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PreviewQuality {
    Full,
    Half,
    Quarter,
}

impl From<PreviewQuality> for engine::render::PreviewQuality {
    fn from(quality: PreviewQuality) -> Self {
        match quality {
            PreviewQuality::Full => Self::Full,
            PreviewQuality::Half => Self::Half,
            PreviewQuality::Quarter => Self::Quarter,
        }
    }
}

/// The preview's zoom (`setPreviewView`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewZoom {
    /// The whole canvas fitted into the preview area (the default).
    Fit,
    /// Canvas pixels at `percent` % of screen (physical) pixels: 100 is one canvas pixel per
    /// screen pixel. Clamped to `PREVIEW_ZOOM_MIN_PERCENT..=PREVIEW_ZOOM_MAX_PERCENT`.
    Percent { percent: u16 },
}

/// Zoom and pan of the preview (`setPreviewView`, render-graph.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    pub zoom: PreviewZoom,
    /// The canvas point shown at the centre of the preview area, relative to the canvas centre,
    /// in canvas widths: 0 centred, 0.5 the right edge, -0.5 the left edge. Clamped to
    /// -0.5..=0.5; ignored at `fit`. `null` (a float's generated type allows it) counts as 0.
    pub pan_x: Option<f32>,
    /// The same in canvas heights: 0.5 the bottom edge.
    pub pan_y: Option<f32>,
}

impl From<PreviewView> for engine::render::PreviewView {
    fn from(view: PreviewView) -> Self {
        Self {
            zoom: match view.zoom {
                PreviewZoom::Fit => engine::render::PreviewZoom::Fit,
                PreviewZoom::Percent { percent } => engine::render::PreviewZoom::Percent(percent),
            },
            pan: (view.pan_x.unwrap_or(0.0), view.pan_y.unwrap_or(0.0)),
        }
    }
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

/// How the running binary was built. A stable key; the UI translates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    /// The profile of this binary (`debug_assertions` on or off).
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        }
    }
}

/// Static facts for the About dialog (F0.10). They do not change while the app runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    /// The workspace version, e.g. `0.1.0`.
    pub version: String,
    pub build_profile: BuildProfile,
    /// Short git commit hash the binary was built from; `None` when unknown at build time.
    pub commit: Option<String>,
    /// The linked FFmpeg's `av_version_info()`, e.g. `n7.1.1`. Informative only, never parsed.
    pub ffmpeg_version: String,
    /// The linked FFmpeg's licence as it reports it (e.g. `LGPL version 2.1 or later`).
    pub ffmpeg_license: String,
    /// The folder with the log files; `None` when logs go to stdout only.
    pub log_dir: Option<String>,
}

/// A newer release found by `checkForUpdate` (F0.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// The announced version, e.g. `0.2.0`.
    pub version: String,
    /// The running version, e.g. `0.1.0`.
    pub current_version: String,
    /// Publish date as the release states it (RFC 3339), if it states one.
    pub date: Option<String>,
    /// Release notes as written in the release (Markdown or plain text, not translated).
    pub notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use engine::render::{DeviceKind, FallbackReason, GpuInfo};

    use super::*;

    #[test]
    fn preview_controls_have_stable_json_shapes() {
        let view: PreviewView = serde_json::from_value(serde_json::json!({
            "zoom": { "kind": "percent", "percent": 200 },
            "panX": 0.25,
            "panY": null,
        }))
        .unwrap();
        assert_eq!(
            engine::render::PreviewView::from(view),
            engine::render::PreviewView {
                zoom: engine::render::PreviewZoom::Percent(200),
                pan: (0.25, 0.0),
            }
        );
        let fit: PreviewView = serde_json::from_value(serde_json::json!({
            "zoom": { "kind": "fit" }, "panX": 0, "panY": 0,
        }))
        .unwrap();
        assert_eq!(fit.zoom, PreviewZoom::Fit);
        assert_eq!(
            serde_json::to_value(PreviewQuality::Quarter).unwrap(),
            "quarter"
        );

        let media = MediaId::new();
        let target = PlaybackTarget::Media { media };
        let json = serde_json::to_value(target).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "media", "media": media.to_string() })
        );
        assert_eq!(
            PlaybackTarget::from(engine::playback::PlaybackTarget::from(target)),
            target
        );
        assert_eq!(
            serde_json::to_value(PlaybackTarget::Timeline).unwrap(),
            serde_json::json!({ "kind": "timeline" })
        );
    }

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
            speed: app_core::Speed::new(250).unwrap(),
            reverse: true,
        };
        project.tracks[0].clips.push(clip);
        let mut freeze = Clip::color("F", Time(10), Time(10), app_core::Rgba8::BLACK);
        freeze.source = ClipSource::Freeze {
            media: video,
            source_time: Time(7),
        };
        project.tracks[0].clips.push(freeze);
        assert_eq!(project.validate(), Ok(()));

        let view = ProjectState::new(
            &Document::new(project.clone()),
            1,
            None,
            false,
            &crate::media_status::MediaStatuses::default(),
        )
        .project;
        assert_eq!(
            view.media[0].used_by, 2,
            "the freeze frame uses the video too"
        );
        assert_eq!(view.media[1].used_by, 0);
        assert_eq!(view.main_track, project.main_track);
        assert!(view.magnetic);
        let clip = &view.tracks[0].clips[0];
        assert_eq!(clip.source_duration, Some(Time::from_seconds(5)));
        assert_eq!(clip.source_end, Some(Time(30)), "10 flicks at 2.5x from 5");
        let json = serde_json::to_value(clip).unwrap();
        assert_eq!(
            json["source"],
            serde_json::json!({
                "kind": "media", "media": video, "sourceIn": 5, "speed": 250, "reverse": true
            })
        );
        assert_eq!(json["sourceEnd"], 30);
        let freeze = serde_json::to_value(&view.tracks[0].clips[1]).unwrap();
        assert_eq!(
            freeze["source"],
            serde_json::json!({ "kind": "freeze", "media": video, "sourceTime": 7 })
        );
        assert_eq!(freeze["sourceEnd"], serde_json::Value::Null);
        assert_eq!(freeze["sourceDuration"], Time::from_seconds(5).flicks());
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

    #[test]
    fn job_stages_serialize_as_stable_keys() {
        let cases = [
            (JobStage::Probing, "probing"),
            (JobStage::GeneratingPreviews, "generatingPreviews"),
            (JobStage::CheckingMedia, "checkingMedia"),
            (JobStage::SearchingFolder, "searchingFolder"),
            (JobStage::ProbingEncoders, "probingEncoders"),
            (JobStage::ExportPreparing, "exportPreparing"),
            (JobStage::ExportEncoding, "exportEncoding"),
            (JobStage::ExportFinalizing, "exportFinalizing"),
        ];
        for (stage, key) in cases {
            assert_eq!(serde_json::to_value(stage).unwrap(), key);
        }
    }

    #[test]
    fn encoder_keys_match_the_engine_and_the_env_variable() {
        use engine::export::encoders::EncoderKind as Engine;
        for kind in Engine::ALL {
            let view = EncoderKind::from(kind);
            assert_eq!(serde_json::to_value(view).unwrap(), kind.key());
            assert_eq!(Engine::from(view), kind);
        }
        for (codec, key) in [(VideoCodec::H264, "h264"), (VideoCodec::Hevc, "hevc")] {
            assert_eq!(serde_json::to_value(codec).unwrap(), key);
            assert_eq!(
                VideoCodec::from(engine::export::plan::VideoCodec::from(codec)),
                codec
            );
        }
    }

    #[test]
    fn capabilities_list_every_candidate_in_order() {
        use engine::export::encoders::{
            EncoderError, EncoderFilter, EncoderKind as Engine, OptionSet, ProbeReport,
        };
        let report = ProbeReport::run_with(&EncoderFilter::default(), |kind, _| match kind {
            Engine::Nvenc | Engine::OpenH264 => Ok(OptionSet::Full),
            _ => Err(EncoderError::NotInBuild("fake")),
        });
        let caps = ExportCapabilities::from(&report);
        assert!(caps.probed);
        assert_eq!(caps.encoders.len(), 10);
        assert_eq!(
            caps.encoders[0],
            EncoderStatus {
                codec: VideoCodec::H264,
                encoder: EncoderKind::Nvenc,
                hardware: true,
                available: true,
            }
        );
        assert_eq!(
            serde_json::to_value(caps.encoders[5]).unwrap(),
            serde_json::json!({
                "codec": "h264",
                "encoder": "openH264",
                "hardware": false,
                "available": true,
            })
        );
        assert!(!caps.encoders[1].available);
        assert_eq!(caps.encoders[6].codec, VideoCodec::Hevc);
        assert_eq!(
            serde_json::to_value(ExportCapabilities::PENDING).unwrap(),
            serde_json::json!({ "probed": false, "encoders": [] })
        );
    }
}
