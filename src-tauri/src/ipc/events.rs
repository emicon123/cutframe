use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use super::IpcError;
use super::types::{AppInfo, JobId, JobStage, MediaFileResult, ProjectState};

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

/// The GPU or the preview mode changed at runtime: the GPU was lost and the preview restarted
/// (`gpu.fallback` is `deviceLost`), or it gave up (`previewMode` is `unavailable`). Replaces the
/// `AppInfo` from `getAppInfo`. Rare.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct AppInfoChanged(pub AppInfo);
