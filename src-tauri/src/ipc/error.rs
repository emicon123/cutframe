use serde::{Deserialize, Serialize};

use app_core::command::CommandError;

/// Error returned by every fallible command. Serialized as `{ kind, message }`.
///
/// `kind` is the stable key the UI branches on and translates; a variant is never renamed
/// (ADR-0003 §4-5). `message` is English diagnostic text for logs only and is never shown to the
/// user.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum IpcError {
    /// The edit was rejected (e.g. unknown track, empty name). The project is unchanged.
    #[error("{0}")]
    InvalidEdit(String),
    /// A subsystem is not available (e.g. no GPU).
    #[error("{0}")]
    Unavailable(String),
    /// A bug or an unexpected failure in the backend.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI translates `kind`; renaming a kind breaks its translations.
    #[test]
    fn kinds_serialize_as_stable_keys() {
        let cases = [
            (IpcError::InvalidEdit("x".into()), "invalidEdit"),
            (IpcError::Unavailable("x".into()), "unavailable"),
            (IpcError::Internal("x".into()), "internal"),
        ];
        for (error, kind) in cases {
            assert_eq!(
                serde_json::to_value(error).unwrap(),
                serde_json::json!({ "kind": kind, "message": "x" })
            );
        }
    }
}
