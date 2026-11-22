//! Why a media file could not be fingerprinted or probed.

use std::fmt;

/// The reason class of a [`MediaError`]. The shell turns it into the stable IPC key the UI
/// translates (`MediaErrorKind` in `src-tauri/src/ipc/types.rs`, design §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaErrorKind {
    FileNotFound,
    AccessDenied,
    /// The path cannot be handed to FFmpeg (not valid UTF-8, contains NUL) or is not a file.
    UnsupportedPath,
    /// No demuxer recognises the file, or it has no video or audio stream.
    UnsupportedFormat,
    /// A stream exists but this FFmpeg build cannot decode it.
    UnsupportedCodec,
    /// The demuxer or decoder fails on the first frames, the file is empty, or the duration is
    /// unknown.
    Damaged,
    /// The job's cancel flag was set while FFmpeg was working.
    Cancelled,
}

/// A failed fingerprint or probe: the kind plus English detail for the log (never shown as is).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct MediaError {
    pub kind: MediaErrorKind,
    pub detail: String,
}

impl MediaError {
    pub fn new(kind: MediaErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    /// Maps an I/O error from opening or reading the file.
    pub fn from_io(err: &std::io::Error) -> Self {
        use std::io::ErrorKind as E;
        let kind = match err.kind() {
            E::NotFound => MediaErrorKind::FileNotFound,
            E::PermissionDenied => MediaErrorKind::AccessDenied,
            E::IsADirectory | E::InvalidFilename => MediaErrorKind::UnsupportedPath,
            _ => MediaErrorKind::Damaged,
        };
        Self::new(kind, err.to_string())
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}
