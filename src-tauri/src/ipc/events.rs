use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use super::types::ProjectState;

/// Emitted after every change to the project (edit, undo, redo, load).
/// The UI keeps the payload with the highest `revision` and drops older ones.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct ProjectChanged(pub ProjectState);
