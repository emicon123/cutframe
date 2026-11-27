//! GPU rendering with wgpu: the render graph of `docs/design/render-graph.md`.
//!
//! [`build_plan`] (pure) says what a project frame contains; [`Compositor`] draws it into an
//! `Rgba16Float` canvas. The preview presents the canvas into its window
//! (`crate::preview`), [`OfflineRenderer`] reads it back or hands it to an encoder.

pub mod compositor;
pub(crate) mod convert;
#[cfg(test)]
mod convert_parity;
pub mod frames;
mod gpu;
pub mod offline;
mod offscreen;
pub mod plan;
pub(crate) mod present;
#[cfg(test)]
mod timing;

pub use compositor::{
    CANVAS_FORMAT, CacheBudget, Compositor, CompositorConfig, MissingLayers, RenderOutcome,
    create_canvas,
};
pub use convert::{AlphaMode, ConvertParams, TransferPath};
pub use frames::{
    FetchPolicy, FrameId, FrameLookup, FrameProvider, FrameRef, Pace, PlaceholderKind,
};
pub use gpu::{AdapterMode, DeviceKind, FallbackReason, GpuContext, GpuError, GpuInfo};
pub use offline::{
    FailedMedia, OfflineOptions, OfflineRenderer, OutputFormat, RenderError, RenderedFrame,
};
pub(crate) use offscreen::Readback;
pub use offscreen::{OUTPUT_FORMAT, OffscreenFrame};
pub use plan::{
    Affine2, BackgroundPlan, LayerContent, LayerPlan, RenderPlan, build_plan, render_size,
};
