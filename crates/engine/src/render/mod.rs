//! GPU rendering with wgpu.

mod gpu;
mod offscreen;
mod test_pattern;

pub use gpu::{AdapterMode, DeviceKind, FallbackReason, GpuContext, GpuError, GpuInfo};
pub use offscreen::{OffscreenFrame, render_offscreen};
pub use test_pattern::{PatternParams, TestPattern};

/// Format of offscreen frames handed to the UI or encoders: 8-bit RGBA, sRGB-encoded.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
