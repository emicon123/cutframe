//! Rendering, media and audio engine. Knows nothing about Tauri.
//!
//! Stage 0 contains the GPU context, a test-pattern renderer (on-screen and offscreen) and the
//! preview thread. Media decoding, the render graph and audio come next.

pub mod preview;
pub mod render;
