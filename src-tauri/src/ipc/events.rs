use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use super::types::ProjectState;

/// Emitted after every change to the project (edit, undo, redo, new, open, save).
/// The UI keeps the payload with the highest `revision` and drops older ones.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct ProjectChanged(pub ProjectState);

/// The user tried to close the main window while the project has unsaved changes. The close was
/// stopped: ask Save / Don't save / Cancel, then call `confirmClose` to close or do nothing to stay
/// open. A clean project closes without this event.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct QuitRequested;
