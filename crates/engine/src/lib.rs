//! Rendering, media and audio engine. Knows nothing about Tauri.
//!
//! Contains the GPU context, a test-pattern renderer (on-screen and offscreen), the preview thread
//! (render on change) with its [`transport`] state, the FFmpeg foundation (licence check, encoder
//! discovery, probing, previews) and the colour reference functions of the render graph
//! ([`color`]): transfer curves, the HDR tone map, YCbCr matrices and range expansion, which the
//! source-convert shader is tested against. The render graph, readers and audio come next.

pub mod audio;
pub mod color;
pub mod export;
pub mod media;
pub mod playback;
pub mod preview;
pub mod render;
pub mod transport;
