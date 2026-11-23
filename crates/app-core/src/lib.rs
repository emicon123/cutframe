//! Project model, edit commands and undo/redo history.
//!
//! This crate is pure: no GPU, media, UI or file-system dependencies. Everything here is testable
//! with plain `cargo test`.

pub mod command;
pub mod document;
pub mod media;
pub mod model;
pub mod props;
pub mod serialization;
pub mod time;
pub mod timeline;
pub mod validate;

pub use document::{Document, EditPhase, HistoryPosition};
pub use media::{MediaAsset, MediaId, MediaKind};
pub use model::{
    CanvasBackground, Clip, ClipId, ClipSource, LinkId, Project, ProjectId, ProjectSettings, Rgba8,
    Track, TrackId, TrackKind,
};
pub use props::{
    Animated, AudioProps, CropInsets, Easing, FitMode, Flip, Keyframe, Vec2, VisualProps,
};
pub use time::{FLICKS_PER_SECOND, FrameRate, MAX_IPC_FLICKS, Rational, Time};
pub use validate::ValidationError;

/// Extension of project files, without the leading dot (ADR-0002 amendment §1).
///
/// A working choice tied to the codename: this is the only place it is written. The UI gets it from
/// the generated bindings, so renaming it here renames it everywhere.
pub const PROJECT_FILE_EXTENSION: &str = "cutframe";
