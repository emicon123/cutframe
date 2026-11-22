//! Media through FFmpeg (rsmpeg, FFmpeg 7.1, LGPL shared build, dynamically linked).
//!
//! - [`ffmpeg`]: versions, licence and configure flags of the linked FFmpeg, and the LGPL-only check
//!   the app runs at startup (CLAUDE.md rule 7, review F26).
//! - [`encoders`]: encoder discovery that only ever selects allowlisted, licence-clean encoders.
//! - [`probe`]: opens a media file and reads its stream info.
//!
//! FFmpeg 4+ registers its codecs and formats on its own, so there is no global init step.
//! Opening or probing files is I/O: call it from a worker job, never from an IPC handler (ADR-0004).

pub mod encoders;
pub mod ffmpeg;
pub mod probe;

pub use encoders::{EncoderBackend, EncoderCandidate, EncoderTarget};
pub use ffmpeg::{FfmpegInfo, FfmpegProblem, LibraryInfo};
pub use probe::{MediaInfo, ProbeError, StreamInfo, StreamKind, probe};
