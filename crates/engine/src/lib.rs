//! Rendering, media and audio engine. Knows nothing about Tauri.
//!
//! Contains the GPU context, a test-pattern renderer (on-screen and offscreen), the preview thread
//! and the FFmpeg foundation (licence check, encoder discovery, probing). Decoding, the render graph
//! and audio come next.

pub mod media;
pub mod preview;
pub mod render;
