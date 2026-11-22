use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use super::types::{AppInfo, ProjectState};

/// Emitted after every change to the project (edit, undo, redo, new, open, save).
/// The UI keeps the payload with the highest `revision` and drops older ones.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct ProjectChanged(pub ProjectState);

/// The user tried to close the main window while the project has unsaved changes. The close was
/// stopped: ask Save / Don't save / Cancel, then call `confirmClose` to close or do nothing to stay
/// open. A clean project closes without this event.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct QuitRequested;

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
