//! GPU rendering with wgpu.

pub(crate) mod convert;
#[cfg(test)]
mod convert_parity;
pub mod frames;
mod gpu;
mod offscreen;
pub mod plan;
mod test_pattern;

pub use convert::{AlphaMode, ConvertParams, TransferPath};
pub use frames::{FetchPolicy, FrameId, FrameLookup, FrameProvider, FrameRef, PlaceholderKind};
pub use gpu::{AdapterMode, DeviceKind, FallbackReason, GpuContext, GpuError, GpuInfo};
pub use offscreen::{OffscreenFrame, render_offscreen};
pub use plan::{
    Affine2, BackgroundPlan, LayerContent, LayerPlan, RenderPlan, build_plan, render_size,
};
pub use test_pattern::{PatternParams, TestPattern};

/// Format of offscreen frames handed to the UI or encoders: 8-bit RGBA, sRGB-encoded.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
