use serde::{Deserialize, Serialize};

use app_core::command::CommandError;

/// Error returned by every fallible command. Serialized as `{ kind, message }`.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum IpcError {
    /// The edit was rejected (e.g. unknown track, empty name). The project is unchanged.
    #[error("{0}")]
    InvalidEdit(String),
    /// A subsystem is not available (e.g. no GPU).
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Internal(String),
}

impl From<CommandError> for IpcError {
    fn from(err: CommandError) -> Self {
        IpcError::InvalidEdit(err.to_string())
    }
}

impl From<tauri::Error> for IpcError {
    fn from(err: tauri::Error) -> Self {
        IpcError::Internal(err.to_string())
    }
}
