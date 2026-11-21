//! Project model, edit commands and undo/redo history.
//!
//! This crate is pure: no GPU, media, UI or file-system dependencies. Everything here is testable
//! with plain `cargo test`.

pub mod command;
pub mod document;
pub mod model;
pub mod serialization;
pub mod time;

pub use document::Document;
pub use model::{Clip, ClipId, Project, ProjectId, ProjectSettings, Track, TrackId, TrackKind};
pub use time::{FLICKS_PER_SECOND, FrameRate, Time};
