use serde::{Deserialize, Serialize};

use app_core::command::CommandError;
use app_core::serialization::LoadError;

use crate::project_file::ReadError;

/// Error returned by every fallible command. Serialized as `{ kind, message }`.
///
/// `kind` is the stable key the UI branches on and translates; a variant is never renamed
/// (ADR-0003 §4-5). `message` is English diagnostic text for logs only and is never shown to the
/// user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize, specta::Type)]
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
    /// Open: the file does not exist.
    #[error("{0}")]
    FileNotFound(String),
    /// Open: not a project file (not JSON, not UTF-8, no `schemaVersion`, or the wrong shape).
    #[error("{0}")]
    MalformedProject(String),
    /// Open: the file was written by a newer version of the app.
    #[error("{0}")]
    ProjectTooNew(String),
    /// Open: the file breaks a model invariant (e.g. overlapping clips). It is not repaired.
    #[error("{0}")]
    InvalidProject(String),
    /// Any other file-system failure: permission denied, disk full, file locked.
    #[error("{0}")]
    Io(String),
    /// The job was cancelled with `cancel_job`; only used as a `job-finished` result.
    #[error("{0}")]
    Cancelled(String),
    /// The edit would put a clip over another clip (a move or drop into an occupied spot). The
    /// project is unchanged; during a drag the last valid state stays visible.
    #[error("{0}")]
    Overlap(String),
    /// The edit would change a clip on a locked track. The project is unchanged.
    #[error("{0}")]
    TrackLocked(String),
    /// `replace_clip_media`: the new media is shorter than the clip. The project is unchanged.
    #[error("{0}")]
    MediaTooShort(String),
    /// Update: the downloaded update or its signature did not verify against the app's updater
    /// key (or the file is not an installer). Nothing was installed.
    #[error("{0}")]
    UpdateRejected(String),
    /// Export: the target file exists and the call did not ask to overwrite it. Nothing was
    /// written; ask the user, then call again with `overwrite`.
    #[error("{0}")]
    FileExists(String),
    /// Only one of these may run at a time and one is running (an export).
    #[error("{0}")]
    Busy(String),
}

impl IpcError {
    /// A failed project-file write or another file-system error.
    pub fn io(err: &std::io::Error) -> Self {
        IpcError::Io(err.to_string())
    }
}

impl From<ReadError> for IpcError {
    fn from(err: ReadError) -> Self {
        let message = err.to_string();
        match err {
            ReadError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                IpcError::FileNotFound(message)
            }
            // `read_to_string` reports a file that is not UTF-8 as invalid data.
            ReadError::Io(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                IpcError::MalformedProject(message)
            }
            ReadError::Io(_) => IpcError::Io(message),
            ReadError::Load(LoadError::Json(_) | LoadError::MissingVersion) => {
                IpcError::MalformedProject(message)
            }
            ReadError::Load(LoadError::TooNew { .. }) => IpcError::ProjectTooNew(message),
            ReadError::Load(LoadError::Invalid(_) | LoadError::Migration { .. }) => {
                IpcError::InvalidProject(message)
            }
        }
    }
}

impl From<CommandError> for IpcError {
    fn from(err: CommandError) -> Self {
        let message = err.to_string();
        match err {
            CommandError::Overlap => IpcError::Overlap(message),
            CommandError::TrackLocked(_) => IpcError::TrackLocked(message),
            CommandError::MediaTooShort => IpcError::MediaTooShort(message),
            _ => IpcError::InvalidEdit(message),
        }
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
            (IpcError::FileNotFound("x".into()), "fileNotFound"),
            (IpcError::MalformedProject("x".into()), "malformedProject"),
            (IpcError::ProjectTooNew("x".into()), "projectTooNew"),
            (IpcError::InvalidProject("x".into()), "invalidProject"),
            (IpcError::Io("x".into()), "io"),
            (IpcError::Cancelled("x".into()), "cancelled"),
            (IpcError::Overlap("x".into()), "overlap"),
            (IpcError::TrackLocked("x".into()), "trackLocked"),
            (IpcError::MediaTooShort("x".into()), "mediaTooShort"),
            (IpcError::UpdateRejected("x".into()), "updateRejected"),
            (IpcError::FileExists("x".into()), "fileExists"),
            (IpcError::Busy("x".into()), "busy"),
        ];
        for (error, kind) in cases {
            assert_eq!(
                serde_json::to_value(error).unwrap(),
                serde_json::json!({ "kind": kind, "message": "x" })
            );
        }
    }

    fn kind(error: IpcError) -> String {
        serde_json::to_value(error).unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn command_errors_the_ui_branches_on_have_their_own_kinds() {
        let cases = [
            (CommandError::Overlap, "overlap"),
            (
                CommandError::TrackLocked(app_core::TrackId::new()),
                "trackLocked",
            ),
            (CommandError::MediaTooShort, "mediaTooShort"),
            (CommandError::InvalidLink, "invalidEdit"),
            (CommandError::NothingToEdit, "invalidEdit"),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(error.into()), expected);
        }
    }

    #[test]
    fn read_errors_map_to_their_kinds() {
        use std::io::{Error, ErrorKind};

        use app_core::ValidationError;

        let load = |text: &str| ReadError::Load(app_core::serialization::load(text).unwrap_err());
        let cases = [
            (
                ReadError::Io(Error::from(ErrorKind::NotFound)),
                "fileNotFound",
            ),
            (
                ReadError::Io(Error::from(ErrorKind::InvalidData)),
                "malformedProject",
            ),
            (
                ReadError::Io(Error::from(ErrorKind::PermissionDenied)),
                "io",
            ),
            (load("not json"), "malformedProject"),
            (load("{}"), "malformedProject"),
            (load(r#"{"schemaVersion": 999}"#), "projectTooNew"),
            (
                ReadError::Load(LoadError::Invalid(ValidationError::EmptyName)),
                "invalidProject",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(error.into()), expected);
        }
    }
}
