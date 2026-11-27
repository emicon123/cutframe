//! The offline entry point of the render graph (render-graph.md §4.5): one project frame at a
//! time, every source frame exact (`FetchPolicy::Exact`), through the same [`Compositor`] as the
//! preview. Export (task 0380), stills and the golden-frame tests use it.
//!
//! Deviation from the design's sketch: `OfflineRenderer::new` takes no `MediaPaths`. The media
//! paths come from the project's `MediaAsset`s, which [`OfflineRenderer::render_frame`] reads
//! from the project it is given.

use std::sync::Arc;

use app_core::media::MediaId;
use app_core::{MediaAsset, Project, Time};

use super::GpuContext;
use super::compositor::{Compositor, CompositorConfig, create_canvas};
use super::frames::{FetchPolicy, FrameLookup, FrameProvider, Pace, PlaceholderKind};
use super::offscreen::{OUTPUT_FORMAT, Readback};
use super::plan::{LayerContent, RenderPlan, build_plan};
use super::present::PresentPass;
use crate::media::readers::{DecodePath, ReaderOptions, ReaderPool};

/// Reader limits and thread priority. Export uses `max_readers: 4, max_hw_readers: 2` and
/// below-normal priority so it does not starve the preview (export.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineOptions {
    pub max_readers: usize,
    pub max_hw_readers: usize,
    pub below_normal_priority: bool,
    /// What a layer whose media is offline or does not decode does.
    pub failed_media: FailedMedia,
}

impl Default for OfflineOptions {
    /// The export settings; failed media make [`OfflineRenderer::render_frame`] fail.
    fn default() -> Self {
        Self {
            max_readers: 4,
            max_hw_readers: 2,
            below_normal_priority: true,
            failed_media: FailedMedia::Error,
        }
    }
}

/// How the offline renderer treats media that are offline or fail to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedMedia {
    /// `render_frame` returns [`RenderError::MediaOffline`] or [`RenderError::Decode`].
    Error,
    /// The layer shows the placeholder, like in the preview (an export the user continued
    /// anyway, golden tests).
    Placeholder,
}

/// The output of [`OfflineRenderer::render_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// 8-bit R'G'B'A, BT.1886 / Rec.709, read back to the CPU: golden tests, posters, stills.
    Rgba8,
    /// The `Rgba16Float` canvas kept on the GPU, for an encoder pass that task 0380 adds.
    GpuCanvas,
}

/// A rendered frame. Borrows the renderer, so the canvas cannot be overwritten while in use.
#[derive(Debug)]
pub enum RenderedFrame<'a> {
    /// Tight RGBA8 rows, `size.0 * 4` bytes each, top to bottom. Opaque: the canvas over black.
    Rgba8(&'a [u8]),
    /// The canvas ([`super::compositor::CANVAS_FORMAT`]); its commands are already submitted.
    /// Valid until the next `render_frame`.
    Gpu(&'a wgpu::TextureView),
}

/// Why a frame could not be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    #[error("media {0} is offline")]
    MediaOffline(MediaId),
    #[error("media {0} cannot be decoded")]
    Decode(MediaId),
    #[error("the GPU failed (device lost or read-back error)")]
    Gpu,
}

/// The canvas and output targets of one size, reused for every frame of that size (review
/// F48).
#[derive(Debug)]
struct Targets {
    size: (u32, u32),
    _canvas: wgpu::Texture,
    canvas_view: wgpu::TextureView,
    readback: Option<Readback>,
}

/// Renders project frames one at a time on its own readers. Runs on an export job worker (or a
/// test thread), never on the preview thread.
pub struct OfflineRenderer {
    gpu: Arc<GpuContext>,
    options: OfflineOptions,
    compositor: Compositor,
    output: PresentPass,
    readers: ReaderPool,
    /// What the readers were last given (`ReaderPool::set_project` runs when it changes).
    media: Option<(Vec<MediaAsset>, (u32, u32))>,
    targets: Option<Targets>,
    failures: Vec<(MediaId, PlaceholderKind)>,
}

impl std::fmt::Debug for OfflineRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineRenderer")
            .field("options", &self.options)
            .field("readers", &self.readers)
            .finish_non_exhaustive()
    }
}

impl OfflineRenderer {
    pub fn new(gpu: Arc<GpuContext>, options: OfflineOptions) -> Self {
        let mut readers = ReaderPool::new(
            Some(Arc::clone(&gpu)),
            ReaderOptions {
                max_readers: options.max_readers.max(1),
                max_hw_readers: options.max_hw_readers,
                below_normal_priority: options.below_normal_priority,
            },
            // Failures are returned by `render_frame`; nothing waits for `FrameReady`.
            |_| {},
        );
        // Frames come in order, as if playing at 1x: decode ahead (and prepare the next earlier
        // window of reversed clips); `Exact` lookups never skip frames.
        readers.set_rate(1);
        Self {
            compositor: Compositor::with_config(&gpu, CompositorConfig::EXPORT),
            output: PresentPass::new(&gpu.device, OUTPUT_FORMAT),
            gpu,
            options,
            readers,
            media: None,
            targets: None,
            failures: Vec::new(),
        }
    }

    /// The device this renderer draws on.
    pub fn gpu(&self) -> &Arc<GpuContext> {
        &self.gpu
    }

    /// The decode path of the reader of `media` (diagnostics, hardware tests).
    pub fn decode_path(&self, media: MediaId) -> Option<DecodePath> {
        self.readers.decode_path(media)
    }

    /// Renders project frame `n` at `size` (normally the canvas size; same aspect) and returns
    /// it in `format`. Blocks until every source frame is decoded. Same pixels as the preview at
    /// full quality.
    pub fn render_frame(
        &mut self,
        project: &Project,
        n: i64,
        size: (u32, u32),
        format: OutputFormat,
    ) -> Result<RenderedFrame<'_>, RenderError> {
        let canvas = (project.settings.width, project.settings.height);
        if self
            .media
            .as_ref()
            .is_none_or(|(media, size)| *media != project.media || *size != canvas)
        {
            self.readers.set_project(project);
            self.media = Some((project.media.clone(), canvas));
        }
        let plan = build_plan(
            project,
            Time::from_frames(n, project.settings.frame_rate),
            size,
        );
        if self.options.failed_media == FailedMedia::Error
            && let Some(media) = unknown_media(project, &plan)
        {
            return Err(RenderError::MediaOffline(media));
        }
        self.render_plan(&plan, format)
    }

    /// Renders `plan` with this renderer's readers.
    fn render_plan(
        &mut self,
        plan: &RenderPlan,
        format: OutputFormat,
    ) -> Result<RenderedFrame<'_>, RenderError> {
        let size = (plan.render_size.0.max(1), plan.render_size.1.max(1));
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let (canvas, canvas_view) = create_canvas(&self.gpu.device, size);
            self.targets = Some(Targets {
                size,
                _canvas: canvas,
                canvas_view,
                readback: None,
            });
            self.compositor.invalidate();
        }
        let gpu = Arc::clone(&self.gpu);
        let Some(targets) = self.targets.as_mut() else {
            return Err(RenderError::Gpu);
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offline"),
            });
        self.failures.clear();
        let mut provider = Recording {
            inner: &mut self.readers,
            failures: &mut self.failures,
        };
        self.compositor.render(
            &gpu,
            &mut encoder,
            plan,
            &mut provider,
            &targets.canvas_view,
        );
        let failure = match self.options.failed_media {
            FailedMedia::Error => self.failures.first().copied(),
            FailedMedia::Placeholder => None,
        };
        match format {
            OutputFormat::GpuCanvas => {
                gpu.queue.submit([encoder.finish()]);
                self.compositor.submitted(&gpu);
                self.compositor.release_completed();
                if let Some(failure) = failure {
                    return Err(failure_error(failure));
                }
                Ok(RenderedFrame::Gpu(&targets.canvas_view))
            }
            OutputFormat::Rgba8 => {
                let readback = targets
                    .readback
                    .get_or_insert_with(|| Readback::new(&gpu.device, size));
                self.output.encode(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &targets.canvas_view,
                    size,
                    &readback.view,
                    size,
                );
                readback.encode_copy(&mut encoder);
                let submission = gpu.queue.submit([encoder.finish()]);
                self.compositor.submitted(&gpu);
                let pixels = readback.read(&gpu, submission).map_err(|err| {
                    tracing::error!(%err, "offline render read-back failed");
                    RenderError::Gpu
                });
                // The wait above ran the completion callbacks.
                self.compositor.release_completed();
                let pixels = pixels?;
                if let Some(failure) = failure {
                    return Err(failure_error(failure));
                }
                Ok(RenderedFrame::Rgba8(pixels))
            }
        }
    }
}

fn failure_error((media, kind): (MediaId, PlaceholderKind)) -> RenderError {
    match kind {
        PlaceholderKind::Offline => RenderError::MediaOffline(media),
        PlaceholderKind::DecodeError => RenderError::Decode(media),
    }
}

/// The media of the first clip that the plan draws as a placeholder because its media id is not
/// in the project (an invalid project; `Project::validate` rejects it).
fn unknown_media(project: &Project, plan: &RenderPlan) -> Option<MediaId> {
    plan.layers
        .iter()
        .filter(|layer| matches!(layer.content, LayerContent::Placeholder(_)))
        .find_map(|layer| {
            project
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|clip| clip.id == layer.clip)
                .and_then(|clip| clip.source.media())
        })
}

/// A provider that records which media failed.
struct Recording<'a> {
    inner: &'a mut ReaderPool,
    failures: &'a mut Vec<(MediaId, PlaceholderKind)>,
}

impl Recording<'_> {
    fn record(&mut self, media: MediaId, lookup: FrameLookup) -> FrameLookup {
        if let FrameLookup::Failed(kind) = lookup {
            self.failures.push((media, kind));
        }
        lookup
    }
}

impl FrameProvider for Recording<'_> {
    fn begin_frame(&mut self) {
        self.inner.begin_frame();
    }

    fn video(
        &mut self,
        media: MediaId,
        sample_time: Time,
        pace: Pace,
        policy: FetchPolicy,
    ) -> FrameLookup {
        let lookup = self.inner.video(media, sample_time, pace, policy);
        self.record(media, lookup)
    }

    fn image(&mut self, media: MediaId, policy: FetchPolicy) -> FrameLookup {
        let lookup = self.inner.image(media, policy);
        self.record(media, lookup)
    }
}
