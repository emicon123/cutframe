//! Media through FFmpeg (rsmpeg, FFmpeg 7.1, LGPL shared build, dynamically linked).
//!
//! - [`ffmpeg`]: versions, licence and configure flags of the linked FFmpeg, and the LGPL-only check
//!   the app runs at startup (CLAUDE.md rule 7, review F26).
//! - [`encoders`]: encoder discovery that only ever selects allowlisted, licence-clean encoders.
//! - [`probe`]: opens a media file, decodes its first frames and describes it as model data.
//! - [`fingerprint`]: the content identity of a media file (cache key, relink match).
//! - [`previews`]: poster, filmstrip and waveform generators for the media panel, on top of
//!   [`picture`] (RGB pictures, JPEG), [`still`] (HEIF tile grids) and [`waveform`].
//! - [`cache`]: the content-keyed preview cache on disk.
//! - `d3d12` (Windows): D3D12VA hardware decode on the render device and zero-copy frames.
//! - [`readers`]: the video reader threads and the frame provider of the render graph.
//!
//! FFmpeg 4+ registers its codecs and formats on its own, so there is no global init step.
//! Opening or probing files is I/O: call it from a worker job, never from an IPC handler (ADR-0004).

pub mod cache;
#[cfg(windows)]
mod d3d12;
pub mod decode;
pub mod encoders;
mod error;
pub mod ffmpeg;
pub mod fingerprint;
pub(crate) mod frame;
mod heif;
mod input;
pub mod picture;
pub mod previews;
pub mod probe;
mod reader;
pub mod readers;
pub mod still;
mod tonemap;
pub mod waveform;

#[cfg(windows)]
pub use d3d12::GpuFrame;
pub use encoders::{EncoderBackend, EncoderCandidate, EncoderTarget};
pub use error::{MediaError, MediaErrorKind};
pub use ffmpeg::{FfmpegInfo, FfmpegProblem, LibraryInfo};
pub use fingerprint::{fingerprint, modified_millis};
pub use frame::{CpuFrame, DecodedVideoFrame, FrameData, PlaneBuf, PlaneLayout, SampleDepth};
pub use probe::{Probed, probe, probe_cancellable};

/// File extensions (lowercase, without the dot) that import takes from a folder and the file
/// dialog offers. A file named explicitly is always tried, whatever its extension.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    // video
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mpg", "mpeg", "mts", "m2ts", "3gp",
    // audio
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", // images
    "jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif", "heic", "heif",
];

/// True if `path` has one of [`MEDIA_EXTENSIONS`] (any case).
pub fn has_media_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTENSIONS.iter().any(|m| m.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn media_extensions_are_lowercase_and_match_any_case() {
        assert!(
            MEDIA_EXTENSIONS
                .iter()
                .all(|e| *e == e.to_lowercase() && !e.starts_with('.'))
        );
        assert!(has_media_extension(Path::new("C:/a/IMG_0042.MOV")));
        assert!(has_media_extension(Path::new("photo.HeIc")));
        assert!(!has_media_extension(Path::new("notes.txt")));
        assert!(!has_media_extension(Path::new("mp4")));
    }
}
