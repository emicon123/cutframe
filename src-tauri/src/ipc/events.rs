use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use app_core::Time;
use engine::playback::TransportStatus;
use engine::transport::PlayState;

use super::IpcError;
use super::types::{
    AppInfo, JobId, JobStage, MediaFileResult, PlaybackTarget, ProjectState, TransportStateView,
};

/// Emitted after every change to the project (edit, undo, redo, new, open, save).
/// The UI keeps the payload with the highest `revision` and drops older ones.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct ProjectChanged(pub ProjectState);

/// The user tried to close the main window while the project has unsaved changes. The close was
/// stopped: ask Save / Don't save / Cancel, then call `confirmClose` to close or do nothing to stay
/// open. A clean project closes without this event.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct QuitRequested;

/// Progress of a running job, at most 10 Hz per job, only when it moved. Never emitted after the
/// job's `job-finished`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: JobId,
    pub stage: JobStage,
    /// 0..=1, `None` while the amount of work is not known yet (e.g. folders being listed).
    pub progress: Option<f32>,
}

/// The end of a job. After `cancel_job` the result is `{ Err: { kind: "cancelled" } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct JobFinished {
    pub job_id: JobId,
    pub result: Result<(), IpcError>,
}

/// Per-file results of a media job (import), emitted just before its `job-finished`. Not emitted
/// for a cancelled job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct MediaJobReport {
    pub job_id: JobId,
    pub files: Vec<MediaFileResult>,
}
/// A second launch of the app passed a project path (e.g. a double-click in Explorer). The window
/// was already brought to the front. Open it like File → Open, after the unsaved-changes prompt;
/// ignore it while another file action or prompt is running.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct OpenFileRequested {
    /// Absolute path as given on the command line; not checked (opening reports errors).
    pub path: String,
}

/// The transport (playback.md §8): at most 60 Hz while playing, and once after every change
/// (play, pause, shuttle, seek, step, loop toggle, end of playback, project replaced), so the
/// last state always arrives. Ignore a tick whose `seek` is older than the generation the last
/// `seek` or `stepFrames` returned. `getPlayback` returns the same payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackTick {
    /// Timeline time of the frame on screen (its start) while playing; the playhead while
    /// paused. Between ticks the UI may extrapolate `time + elapsed x rate` for drawing.
    pub time: Time,
    pub state: TransportStateView,
    /// -8..=8 (1, 2, 4, 8 forward, negative reverse), 0 while paused.
    pub rate: i8,
    pub looping: bool,
    /// The newest seek generation the engine has applied.
    pub seek: u32,
    /// Frames dropped since the last play start (skipped or shown late).
    pub dropped_frames: u32,
    pub target: PlaybackTarget,
}

impl PlaybackTick {
    /// The tick of a session without a preview engine: paused at 0.
    pub const IDLE: Self = Self {
        time: Time::ZERO,
        state: TransportStateView::Paused,
        rate: 0,
        looping: false,
        seek: 0,
        dropped_frames: 0,
        target: PlaybackTarget::Timeline,
    };
}

impl From<TransportStatus> for PlaybackTick {
    fn from(status: TransportStatus) -> Self {
        Self {
            time: status.time,
            state: match status.state {
                PlayState::Paused => TransportStateView::Paused,
                PlayState::Prerolling | PlayState::Playing => TransportStateView::Playing,
            },
            rate: status.rate,
            looping: status.looping,
            seek: status.seek_gen,
            dropped_frames: status.dropped_frames,
            target: PlaybackTarget::Timeline,
        }
    }
}

/// The GPU or the preview mode changed at runtime: the GPU was lost and the preview restarted
/// (`gpu.fallback` is `deviceLost`), or it gave up (`previewMode` is `unavailable`). Replaces the
/// `AppInfo` from `getAppInfo`. Rare.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct AppInfoChanged(pub AppInfo);

/// Progress of the download that `installUpdate` started (F0.12). `downloading` comes at most
/// 10 Hz; each download ends with exactly one `ready` or `failed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, Event)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateProgress {
    /// Byte counts are decimal strings (64-bit, ADR-0003 §3). `total` is `None` when the server
    /// does not send a length.
    Downloading {
        downloaded: String,
        total: Option<String>,
    },
    /// Downloaded and the signature verified: `restartApp` installs it and starts the new version.
    Ready,
    /// The download or the verification failed; `installUpdate` may be called again. `error.kind`
    /// is `unavailable` (network), `updateRejected` (bad signature or file) or `io`/`internal`.
    Failed { error: IpcError },
}
