//! The GPU half of the render graph (design `docs/design/render-graph.md` §4.1–§4.3, §6, §7):
//! [`Compositor::render`] executes a [`RenderPlan`] into an `Rgba16Float` canvas. It is the only
//! drawing code for timeline content: the preview, the [`OfflineRenderer`] (export, golden
//! tests) and `--smoke-test` call it and differ only in the fetch policy and the output pass.
//!
//! Per frame:
//! 1. **Fetch.** One [`FrameProvider`] lookup per media layer, with the compositor's
//!    [`FetchPolicy`]. A missing frame (`BestAvailable` only) leaves the layer out, or keeps the
//!    whole canvas ([`MissingLayers::KeepCanvas`], the preview's 250 ms rule of §7).
//! 2. **Skip.** When the canvas, the plan and every frame identity equal the last composite,
//!    nothing is recorded (§6 "last canvas").
//! 3. **Convert** (§4.2 step 2) once per distinct source frame, at its convert size (the
//!    on-canvas size in render pixels, capped at the display size, even; the larger size wins
//!    when two layers share a frame), into a cached `Rgba16Float` texture. Hardware frames are
//!    sampled through their plane views and released in the same encoder
//!    (`GpuFrame::record_release`); their [`FrameRef`]s are held until that submission has
//!    completed ([`Compositor::submitted`]).
//! 4. **Layers** (§4.2 steps 1 and 3): one render pass that clears to the background and draws
//!    each layer bottom to top as a quad (texture, solid colour or placeholder stripes), opacity
//!    applied to the premultiplied colour, blended premultiplied-over.
//!
//! **Caller contract.** Submit the encoder passed to `render` before the next `render` (the
//! layer uniforms and the reused upload textures are written with `Queue::write_*`), then call
//! [`Compositor::submitted`] so the hardware frames it sampled can be released after the GPU is
//! done with them.
//!
//! [`OfflineRenderer`]: super::offline::OfflineRenderer

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use app_core::MediaId;
use bytemuck::{Pod, Zeroable};

use super::GpuContext;
use super::convert::{CONVERT_FORMAT, ConvertParams, ConvertPass, SourceTextures, convert_size};
use super::frames::{FetchPolicy, FrameId, FrameLookup, FrameProvider, FrameRef, PlaceholderKind};
use super::plan::{Affine2, BackgroundPlan, LayerContent, LayerPlan, RenderPlan};
use crate::media::FrameData;

const SHADER: &str = include_str!("shaders/layer.wgsl");

/// Format of the canvas and of every working texture (render-graph.md §5.1): premultiplied
/// R'G'B'A, Rec.709 primaries, BT.1886 encoding.
pub const CANVAS_FORMAT: wgpu::TextureFormat = CONVERT_FORMAT;

/// Bytes per texel of [`CANVAS_FORMAT`].
const TEXEL_BYTES: u64 = 8;

/// Spare textures kept for reuse (converted sources and upload planes).
const SPARE_TEXTURES: usize = 8;

/// The two colours of each placeholder (§4.3), R'G'B' 0..=255, opaque.
const OFFLINE_STRIPES: [[u8; 3]; 2] = [[0x40, 0x40, 0x40], [0x70, 0x70, 0x70]];
const DECODE_ERROR_STRIPES: [[u8; 3]; 2] = [[0x60, 0x20, 0x20], [0xa0, 0x30, 0x30]];
/// Stripe period as a fraction of the canvas's long side.
const STRIPES_PER_LONG_SIDE: f32 = 24.0;

/// Creates a canvas texture of `size` for [`Compositor::render`]: a render target that the
/// output passes sample and that can be copied out.
pub fn create_canvas(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("compositor.canvas"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CANVAS_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Bounds of the source-texture cache (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheBudget {
    /// GPU memory of all converted source textures.
    pub max_bytes: u64,
    /// Converted frames kept per media (`None`: only the byte budget applies).
    pub max_per_media: Option<usize>,
}

impl CacheBudget {
    /// The preview: 256 MB, so paused edits re-composite without converting again.
    pub const PREVIEW: Self = Self {
        max_bytes: 256 << 20,
        max_per_media: None,
    };
    /// Export: frames come in order, so about two frames per source are enough.
    pub const EXPORT: Self = Self {
        max_bytes: 512 << 20,
        max_per_media: Some(2),
    };
}

/// How a compositor fetches frames and how much it caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositorConfig {
    pub policy: FetchPolicy,
    pub budget: CacheBudget,
}

impl CompositorConfig {
    /// The preview: never wait for a frame.
    pub const PREVIEW: Self = Self {
        policy: FetchPolicy::BestAvailable,
        budget: CacheBudget::PREVIEW,
    };
    /// Export, golden tests: wait for every exact frame.
    pub const EXPORT: Self = Self {
        policy: FetchPolicy::Exact,
        budget: CacheBudget::EXPORT,
    };
}

/// What to do when a layer's frame is not decoded yet (`BestAvailable` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingLayers {
    /// Draw the frame without the layer.
    LeaveOut,
    /// Record nothing, so the canvas keeps the previous frame (the preview within 250 ms of the
    /// request, §7: cut points and scrubs do not flash black).
    KeepCanvas,
}

/// The result of [`Compositor::render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderOutcome {
    /// Layers drawn with a stale frame (`BestAvailable`).
    pub stale_layers: u32,
    /// Layers whose frame was missing (`BestAvailable`): left out, or the canvas was kept.
    pub missing_layers: u32,
    /// Whether anything was recorded into the encoder. `false` when the canvas already shows
    /// this plan with these frames, or with [`MissingLayers::KeepCanvas`] and a missing layer.
    pub drawn: bool,
}

/// What one layer of the last composite showed, compared to skip redundant composites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerKey {
    /// Drawn from the plan alone (colour, placeholder in the plan).
    Plan,
    Frame(FrameId),
    Missing,
    Failed(PlaceholderKind),
}

/// A layer after the fetch.
#[derive(Debug)]
enum Resolved {
    /// Draw `targets[index]`'s converted texture.
    Source(usize),
    /// Premultiplied colour.
    Solid([f32; 4]),
    Stripes(PlaceholderKind),
    Skip,
}

/// One distinct source frame of the current composite.
#[derive(Debug)]
struct Target {
    frame: FrameRef,
    params: ConvertParams,
    /// Largest on-canvas size of the layers that use it, then the convert size.
    size: (u32, u32),
    /// The cache entry after the convert; `None` if the frame could not be converted.
    key: Option<CacheKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    frame: FrameId,
    size: (u32, u32),
    params: ConvertParams,
}

#[derive(Debug)]
struct CacheEntry {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Layer bind group of this texture; rebuilt when the uniform buffer is replaced.
    bind_group: Option<wgpu::BindGroup>,
    bytes: u64,
    /// Composite number of the last use.
    last_used: u64,
}

/// The LRU of converted source textures (§6).
#[derive(Debug)]
struct SourceCache {
    budget: CacheBudget,
    entries: HashMap<CacheKey, CacheEntry>,
    bytes: u64,
    /// Textures of evicted entries, reused for new ones of the same size.
    spare: Vec<wgpu::Texture>,
}

impl SourceCache {
    fn new(budget: CacheBudget) -> Self {
        Self {
            budget,
            entries: HashMap::new(),
            bytes: 0,
            spare: Vec::new(),
        }
    }

    fn touch(&mut self, key: &CacheKey, now: u64) -> bool {
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.last_used = now;
                true
            }
            None => false,
        }
    }

    /// A texture for a new entry of `size`: a spare one of that size, else a new one.
    fn texture(&mut self, device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
        if let Some(index) = self
            .spare
            .iter()
            .position(|t| (t.width(), t.height()) == size)
        {
            return self.spare.swap_remove(index);
        }
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("compositor.source"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn insert(&mut self, key: CacheKey, texture: wgpu::Texture, now: u64) {
        let bytes = u64::from(key.size.0) * u64::from(key.size.1) * TEXEL_BYTES;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bytes += bytes;
        let entry = CacheEntry {
            texture,
            view,
            bind_group: None,
            bytes,
            last_used: now,
        };
        if let Some(old) = self.entries.insert(key, entry) {
            self.bytes -= old.bytes;
        }
    }

    /// Evicts least recently used entries that composite `now` does not use, until the budget
    /// holds.
    fn evict(&mut self, now: u64) {
        if let Some(per_media) = self.budget.max_per_media {
            let mut media: Vec<MediaId> = self.entries.keys().map(|k| k.frame.media).collect();
            media.sort_unstable();
            media.dedup();
            for id in media {
                while self.entries.keys().filter(|k| k.frame.media == id).count() > per_media {
                    if !self.evict_one(now, Some(id)) {
                        break;
                    }
                }
            }
        }
        while self.bytes > self.budget.max_bytes {
            if !self.evict_one(now, None) {
                break;
            }
        }
    }

    fn evict_one(&mut self, now: u64, media: Option<MediaId>) -> bool {
        let victim = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                entry.last_used != now && media.is_none_or(|id| key.frame.media == id)
            })
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| *key);
        let Some(key) = victim else {
            return false;
        };
        if let Some(entry) = self.entries.remove(&key) {
            self.bytes -= entry.bytes;
            if self.spare.len() < SPARE_TEXTURES {
                self.spare.push(entry.texture);
            }
        }
        true
    }

    fn forget_bind_groups(&mut self) {
        for entry in self.entries.values_mut() {
            entry.bind_group = None;
        }
    }
}

/// The shader's `Layer` uniform.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct LayerUniforms {
    row0: [f32; 4],
    row1: [f32; 4],
    dest: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    color2: [f32; 4],
    params: [f32; 4],
    mode: [u32; 4],
}

const MODE_TEXTURE: u32 = 0;
const MODE_SOLID: u32 = 1;
const MODE_STRIPES: u32 = 2;

/// The layer pipeline and its per-layer uniforms (one buffer, dynamic offsets).
#[derive(Debug)]
struct LayerPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound for colour and placeholder layers.
    blank: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    /// Bytes between two layers' uniforms (the device's offset alignment).
    stride: u64,
    /// Layers the buffer holds.
    capacity: usize,
    /// The bind group with `blank`.
    solid: wgpu::BindGroup,
    /// CPU copy of the uniforms of one composite, reused.
    staging: Vec<u8>,
}

impl LayerPass {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("layer"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<LayerUniforms>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layer.pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("layer.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: CANVAS_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("layer.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("layer.blank"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CANVAS_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        let stride = (std::mem::size_of::<LayerUniforms>() as u64).div_ceil(alignment) * alignment;
        let capacity = 16;
        let uniforms = Self::buffer(device, stride, capacity);
        let solid = Self::bind_group_for(device, &layout, &uniforms, &sampler, &blank);
        Self {
            pipeline,
            layout,
            sampler,
            blank,
            uniforms,
            stride,
            capacity,
            solid,
            staging: Vec::new(),
        }
    }

    fn buffer(device: &wgpu::Device, stride: u64, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer.uniforms"),
            size: stride * capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn bind_group_for(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniforms: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer.bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<LayerUniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view),
                },
            ],
        })
    }

    fn bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        Self::bind_group_for(device, &self.layout, &self.uniforms, &self.sampler, view)
    }

    /// Makes room for `layers` uniforms. Returns `true` when the buffer was replaced (every
    /// bind group that refers to it must be rebuilt).
    fn reserve(&mut self, device: &wgpu::Device, layers: usize) -> bool {
        if layers <= self.capacity {
            return false;
        }
        self.capacity = layers.next_power_of_two();
        self.uniforms = Self::buffer(device, self.stride, self.capacity);
        self.solid = Self::bind_group_for(
            device,
            &self.layout,
            &self.uniforms,
            &self.sampler,
            &self.blank,
        );
        true
    }
}

/// Frames sampled by one submission, dropped once it has completed.
#[derive(Debug)]
struct Hold {
    done: Arc<AtomicBool>,
    frames: Vec<FrameRef>,
}

/// The last composite (§6 "last canvas").
#[derive(Debug)]
struct Last {
    canvas: wgpu::TextureView,
    plan: RenderPlan,
    keys: Vec<LayerKey>,
}

/// Pipelines, the source-texture cache and the hold list of one consumer (the preview, or one
/// export) on one device. Drop it when the device is lost.
#[derive(Debug)]
pub struct Compositor {
    config: CompositorConfig,
    convert: ConvertPass,
    layers: LayerPass,
    cache: SourceCache,
    /// Upload textures of CPU frames, reused by later composites (never within one).
    spare_uploads: Vec<SourceTextures>,
    used_uploads: Vec<SourceTextures>,
    /// Hardware frames sampled by the encoder of the last `render`, not yet submitted.
    unsubmitted: Vec<FrameRef>,
    holds: Vec<Hold>,
    last: Option<Last>,
    /// Composites so far (the cache's clock).
    count: u64,
    // Per-composite scratch, reused.
    resolved: Vec<Resolved>,
    keys: Vec<LayerKey>,
    targets: Vec<Target>,
    /// (uniform offset, cache key) of each drawn layer.
    draws: Vec<(u32, Option<CacheKey>)>,
}

impl Compositor {
    /// A compositor for the preview ([`CompositorConfig::PREVIEW`]).
    pub fn new(gpu: &GpuContext) -> Self {
        Self::with_config(gpu, CompositorConfig::PREVIEW)
    }

    pub fn with_config(gpu: &GpuContext, config: CompositorConfig) -> Self {
        Self {
            config,
            convert: ConvertPass::new(&gpu.device),
            layers: LayerPass::new(&gpu.device),
            cache: SourceCache::new(config.budget),
            spare_uploads: Vec::new(),
            used_uploads: Vec::new(),
            unsubmitted: Vec::new(),
            holds: Vec::new(),
            last: None,
            count: 0,
            resolved: Vec::new(),
            keys: Vec::new(),
            targets: Vec::new(),
            draws: Vec::new(),
        }
    }

    pub fn config(&self) -> CompositorConfig {
        self.config
    }

    /// Draws `plan` into `canvas`, an [`CANVAS_FORMAT`] view of `plan.render_size`, leaving
    /// missing layers out. See the module docs for the caller contract.
    pub fn render(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        plan: &RenderPlan,
        frames: &mut dyn FrameProvider,
        canvas: &wgpu::TextureView,
    ) -> RenderOutcome {
        self.render_with(gpu, encoder, plan, frames, canvas, MissingLayers::LeaveOut)
    }

    /// [`Self::render`] with a choice for missing layers.
    pub fn render_with(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        plan: &RenderPlan,
        frames: &mut dyn FrameProvider,
        canvas: &wgpu::TextureView,
        missing: MissingLayers,
    ) -> RenderOutcome {
        // Frames of an encoder the caller did not report: it is submitted by now (contract).
        self.submitted(gpu);
        self.release_completed();
        let outcome = self.fetch(plan, frames);
        let keep = outcome.missing_layers > 0 && missing == MissingLayers::KeepCanvas;
        let unchanged = self.last.as_ref().is_some_and(|last| {
            last.canvas == *canvas && last.plan == *plan && last.keys == self.keys
        });
        if keep || unchanged {
            self.resolved.clear();
            self.targets.clear();
            return outcome;
        }
        self.count += 1;
        self.convert_sources(gpu, encoder);
        self.draw_layers(gpu, encoder, plan, canvas);
        self.cache.evict(self.count);
        self.spare_uploads.append(&mut self.used_uploads);
        if self.spare_uploads.len() > SPARE_TEXTURES {
            let excess = self.spare_uploads.len() - SPARE_TEXTURES;
            self.spare_uploads.drain(..excess);
        }
        self.resolved.clear();
        self.targets.clear();
        let keys = std::mem::take(&mut self.keys);
        match &mut self.last {
            Some(last) => {
                last.canvas = canvas.clone();
                last.plan.clone_from(plan);
                self.keys = std::mem::replace(&mut last.keys, keys);
            }
            None => {
                self.last = Some(Last {
                    canvas: canvas.clone(),
                    plan: plan.clone(),
                    keys,
                });
            }
        }
        RenderOutcome {
            drawn: true,
            ..outcome
        }
    }

    /// Call right after submitting the encoder of the last [`Self::render`]: the hardware frames
    /// it sampled are dropped once that submission has completed (seen by a later
    /// `Device::poll` or `Queue::submit`, then [`Self::release_completed`] or the next render).
    pub fn submitted(&mut self, gpu: &GpuContext) {
        if self.unsubmitted.is_empty() {
            return;
        }
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        gpu.queue
            .on_submitted_work_done(move || flag.store(true, Ordering::Release));
        self.holds.push(Hold {
            done,
            frames: std::mem::take(&mut self.unsubmitted),
        });
    }

    /// Drops the held hardware frames whose submission has completed.
    pub fn release_completed(&mut self) {
        self.holds.retain(|hold| !hold.done.load(Ordering::Acquire));
    }

    /// Hardware frames held until their submission completes (diagnostics, tests).
    pub fn held_frames(&self) -> usize {
        self.unsubmitted.len() + self.holds.iter().map(|h| h.frames.len()).sum::<usize>()
    }

    /// Forgets the last composite, so the next render draws even if nothing changed (the
    /// canvas was overwritten or recreated by its owner).
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    /// Step 1: one lookup per layer into `resolved`, `keys` and `targets`.
    fn fetch(&mut self, plan: &RenderPlan, frames: &mut dyn FrameProvider) -> RenderOutcome {
        self.resolved.clear();
        self.keys.clear();
        self.targets.clear();
        let mut outcome = RenderOutcome::default();
        let policy = self.config.policy;
        frames.begin_frame();
        for layer in &plan.layers {
            let (lookup, params) = match &layer.content {
                LayerContent::Video {
                    media,
                    sample_time,
                    pace,
                    convert,
                } => (frames.video(*media, *sample_time, *pace, policy), *convert),
                LayerContent::Image { media, convert } => (frames.image(*media, policy), *convert),
                LayerContent::Color(color) => {
                    self.resolved.push(Resolved::Solid(premultiplied(
                        [color.r, color.g, color.b],
                        color.a,
                    )));
                    self.keys.push(LayerKey::Plan);
                    continue;
                }
                LayerContent::Placeholder(kind) => {
                    self.resolved.push(Resolved::Stripes(*kind));
                    self.keys.push(LayerKey::Plan);
                    continue;
                }
            };
            let frame = match lookup {
                FrameLookup::Exact(frame) => frame,
                FrameLookup::Stale(frame) => {
                    outcome.stale_layers += 1;
                    frame
                }
                FrameLookup::Missing => {
                    outcome.missing_layers += 1;
                    self.resolved.push(Resolved::Skip);
                    self.keys.push(LayerKey::Missing);
                    continue;
                }
                FrameLookup::Failed(kind) => {
                    self.resolved.push(Resolved::Stripes(kind));
                    self.keys.push(LayerKey::Failed(kind));
                    continue;
                }
            };
            self.keys.push(LayerKey::Frame(frame.id()));
            let size = on_canvas_size(layer, plan);
            let index = match self
                .targets
                .iter()
                .position(|t| t.frame.id() == frame.id() && t.params == params)
            {
                Some(index) => {
                    let target = &mut self.targets[index];
                    target.size = (target.size.0.max(size.0), target.size.1.max(size.1));
                    index
                }
                None => {
                    self.targets.push(Target {
                        frame,
                        params,
                        size,
                        key: None,
                    });
                    self.targets.len() - 1
                }
            };
            self.resolved.push(Resolved::Source(index));
        }
        outcome
    }

    /// Step 3: converts every target that is not cached.
    fn convert_sources(&mut self, gpu: &GpuContext, encoder: &mut wgpu::CommandEncoder) {
        let now = self.count;
        for index in 0..self.targets.len() {
            let target = &self.targets[index];
            let size = convert_size(target.size, target.params.display_size);
            let key = CacheKey {
                frame: target.frame.id(),
                size,
                params: target.params,
            };
            if self.cache.touch(&key, now) {
                self.targets[index].key = Some(key);
                continue;
            }
            let source = match target.frame.data() {
                FrameData::Cpu(cpu) => {
                    let reuse = self
                        .spare_uploads
                        .iter()
                        .position(|spare| spare.fits(cpu))
                        .map(|i| self.spare_uploads.swap_remove(i));
                    match SourceTextures::upload(&gpu.device, &gpu.queue, cpu, reuse) {
                        Ok(source) => source,
                        Err(err) => {
                            tracing::warn!(
                                media = %key.frame.media, %err,
                                "cannot upload a decoded frame; showing the placeholder"
                            );
                            continue;
                        }
                    }
                }
                #[cfg(windows)]
                FrameData::Gpu(surface) => gpu_source(surface),
            };
            let texture = self.cache.texture(&gpu.device, size);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.convert
                .encode(&gpu.device, encoder, &source, &target.params, &view, size);
            #[cfg(windows)]
            if let FrameData::Gpu(surface) = target.frame.data() {
                // The convert was the surface's last use in this encoder (§5.4 release state).
                surface.record_release(encoder);
                self.unsubmitted.push(target.frame.clone());
            }
            if source.is_owned() {
                self.used_uploads.push(source);
            }
            self.cache.insert(key, texture, now);
            self.targets[index].key = Some(key);
        }
    }

    /// Step 4: background and layers in one pass.
    fn draw_layers(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        plan: &RenderPlan,
        canvas: &wgpu::TextureView,
    ) {
        let device = &gpu.device;
        if self.layers.reserve(device, plan.layers.len()) {
            self.cache.forget_bind_groups();
        }
        let render = (
            plan.render_size.0.max(1) as f32,
            plan.render_size.1.max(1) as f32,
        );
        let scale = (
            render.0 / plan.canvas.0.max(1) as f32,
            render.1 / plan.canvas.1.max(1) as f32,
        );
        let period = (plan.canvas.0.max(plan.canvas.1) as f32 / STRIPES_PER_LONG_SIDE)
            .round()
            .max(2.0);
        let stride = self.layers.stride as usize;
        self.layers.staging.clear();
        self.layers
            .staging
            .resize(stride * plan.layers.len().max(1), 0);
        self.draws.clear();
        for (slot, (layer, resolved)) in plan.layers.iter().zip(&self.resolved).enumerate() {
            let (mode, color, color2, key) = match resolved {
                Resolved::Skip => continue,
                Resolved::Solid(color) => (MODE_SOLID, *color, [0.0; 4], None),
                Resolved::Stripes(kind) => {
                    let [a, b] = stripes(*kind);
                    (MODE_STRIPES, a, b, None)
                }
                Resolved::Source(index) => match self.targets[*index].key {
                    Some(key) => (MODE_TEXTURE, [0.0; 4], [0.0; 4], Some(key)),
                    None => {
                        let [a, b] = stripes(PlaceholderKind::DecodeError);
                        (MODE_STRIPES, a, b, None)
                    }
                },
            };
            let crop = layer.crop;
            let uniforms = LayerUniforms {
                row0: scaled_row(layer.transform, 0, scale.0),
                row1: scaled_row(layer.transform, 1, scale.1),
                dest: [render.0, render.1, scale.0, scale.1],
                uv: [crop.left, crop.top, 1.0 - crop.right, 1.0 - crop.bottom],
                color,
                color2,
                params: [layer.opacity.clamp(0.0, 1.0), period, 0.0, 0.0],
                mode: [mode, 0, 0, 0],
            };
            let offset = slot * stride;
            self.layers.staging[offset..offset + std::mem::size_of::<LayerUniforms>()]
                .copy_from_slice(bytemuck::bytes_of(&uniforms));
            self.draws.push(((slot * stride) as u32, key));
        }
        gpu.queue
            .write_buffer(&self.layers.uniforms, 0, &self.layers.staging);
        for (_, key) in &self.draws {
            if let Some(key) = key
                && let Some(entry) = self.cache.entries.get_mut(key)
                && entry.bind_group.is_none()
            {
                entry.bind_group = Some(self.layers.bind_group(device, &entry.view));
            }
        }

        let BackgroundPlan::Color(background) = plan.background;
        let [r, g, b, a] = premultiplied([background.r, background.g, background.b], background.a);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compositor.layers"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: canvas,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(r),
                        g: f64::from(g),
                        b: f64::from(b),
                        a: f64::from(a),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.layers.pipeline);
        for (offset, key) in &self.draws {
            let bind_group = key
                .and_then(|key| self.cache.entries.get(&key))
                .and_then(|entry| entry.bind_group.as_ref())
                .unwrap_or(&self.layers.solid);
            pass.set_bind_group(0, bind_group, &[*offset]);
            pass.draw(0..6, 0..1);
        }
    }
}

/// The planes of a zero-copy hardware frame: `Plane0` / `Plane1` views of the coded surface,
/// with the visible picture inside it (§5.4 "Surface size").
#[cfg(windows)]
pub(crate) fn gpu_source(surface: &crate::media::GpuFrame) -> SourceTextures {
    use super::convert::SourcePlane;
    use crate::media::{PlaneLayout, SampleDepth};

    let [luma, chroma] = surface.plane_views();
    let depth = if surface.texture.format() == wgpu::TextureFormat::P010 {
        SampleDepth::U16 {
            bits: 10,
            msb_aligned: true,
        }
    } else {
        SampleDepth::U8
    };
    let (w, h) = surface.coded;
    SourceTextures::from_planes(
        PlaneLayout::SemiPlanar,
        depth,
        surface.visible,
        [
            Some(SourcePlane {
                view: luma,
                size: (w, h),
            }),
            Some(SourcePlane {
                view: chroma,
                size: (w.div_ceil(2), h.div_ceil(2)),
            }),
            None,
        ],
    )
}

/// The layer's size on the canvas in render pixels: the lengths of the transformed unit quad's
/// edges, divided by the visible fraction after crop, rounded up.
fn on_canvas_size(layer: &LayerPlan, plan: &RenderPlan) -> (u32, u32) {
    let sx = plan.render_size.0 as f32 / plan.canvas.0.max(1) as f32;
    let sy = plan.render_size.1 as f32 / plan.canvas.1.max(1) as f32;
    let [x, y] = layer.transform.rows;
    let width = (x[0] * sx).hypot(y[0] * sy);
    let height = (x[1] * sx).hypot(y[1] * sy);
    let crop = layer.crop;
    let visible_w = (1.0 - crop.left - crop.right).max(1e-3);
    let visible_h = (1.0 - crop.top - crop.bottom).max(1e-3);
    let side = |v: f32| v.ceil().clamp(1.0, 16384.0) as u32;
    (side(width / visible_w), side(height / visible_h))
}

/// Row `row` of `transform` in render pixels.
fn scaled_row(transform: Affine2, row: usize, scale: f32) -> [f32; 4] {
    let [a, b, c] = transform.rows[row];
    [a * scale, b * scale, c * scale, 0.0]
}

/// A straight-alpha 8-bit colour as premultiplied floats.
fn premultiplied(rgb: [u8; 3], alpha: u8) -> [f32; 4] {
    let a = f32::from(alpha) / 255.0;
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0 * a);
    [r, g, b, a]
}

/// The two premultiplied colours of a placeholder.
fn stripes(kind: PlaceholderKind) -> [[f32; 4]; 2] {
    let colors = match kind {
        PlaceholderKind::Offline => OFFLINE_STRIPES,
        PlaceholderKind::DecodeError => DECODE_ERROR_STRIPES,
    };
    colors.map(|rgb| premultiplied(rgb, 255))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::plan::Affine2;

    fn plan(canvas: (u32, u32), render: (u32, u32), transform: Affine2) -> (RenderPlan, LayerPlan) {
        let layer = LayerPlan {
            clip: app_core::ClipId::new(),
            content: LayerContent::Color(app_core::Rgba8::BLACK),
            transform,
            crop: Default::default(),
            opacity: 1.0,
        };
        let plan = RenderPlan {
            frame: 0,
            time: app_core::Time::ZERO,
            canvas,
            render_size: render,
            background: BackgroundPlan::Color(app_core::Rgba8::BLACK),
            layers: vec![layer.clone()],
        };
        (plan, layer)
    }

    #[test]
    fn on_canvas_size_is_in_render_pixels() {
        let (p, l) = plan(
            (1920, 1080),
            (960, 540),
            Affine2::rect(656.25, 0.0, 607.5, 1080.0),
        );
        assert_eq!(on_canvas_size(&l, &p), (304, 540));
        let (p, l) = plan(
            (1920, 1080),
            (1920, 1080),
            Affine2::rect(0.0, 0.0, 1920.0, 1080.0),
        );
        assert_eq!(on_canvas_size(&l, &p), (1920, 1080));
        // A quarter turn swaps nothing: the u edge is still the source's width.
        let rotated = Affine2 {
            rows: [[0.0, -100.0, 200.0], [50.0, 0.0, 0.0]],
        };
        let (p, l) = plan((400, 400), (400, 400), rotated);
        assert_eq!(on_canvas_size(&l, &p), (50, 100));
    }

    #[test]
    fn colours_are_premultiplied() {
        assert_eq!(premultiplied([255, 0, 51], 255), [1.0, 0.0, 0.2, 1.0]);
        let [r, g, b, a] = premultiplied([255, 255, 255], 0);
        assert_eq!([r, g, b, a], [0.0; 4]);
        let half = premultiplied([255, 0, 0], 51);
        assert!((half[0] - 0.2).abs() < 1e-6 && (half[3] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn the_cache_evicts_least_recently_used_entries_outside_the_current_composite() {
        let budget = CacheBudget {
            max_bytes: 3 * 4 * 4 * TEXEL_BYTES,
            max_per_media: Some(2),
        };
        let mut cache = SourceCache::new(budget);
        let gpu = crate::render::GpuContext::new_with(crate::render::AdapterMode::SoftwareOnly)
            .expect("WARP");
        let media = MediaId::new();
        let other = MediaId::new();
        let key = |media, pts| CacheKey {
            frame: FrameId {
                media,
                source: 1,
                pts: Some(app_core::Time(pts)),
            },
            size: (4, 4),
            params: crate::render::ConvertParams {
                matrix: None,
                range: crate::color::ColorRange::Full,
                transfer: crate::render::TransferPath::Sdr,
                rotation: app_core::media::Rotation::None,
                alpha: crate::render::AlphaMode::Opaque,
                display_size: (4, 4),
            },
        };
        for (now, pts) in [(1, 0), (2, 1), (3, 2)] {
            let texture = cache.texture(&gpu.device, (4, 4));
            cache.insert(key(media, pts), texture, now);
            cache.evict(now);
        }
        // Two per media: the oldest frame of `media` went, its texture is spare.
        assert!(!cache.entries.contains_key(&key(media, 0)));
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.spare.len(), 1);
        let texture = cache.texture(&gpu.device, (4, 4));
        assert!(cache.spare.is_empty(), "the spare texture is reused");
        cache.insert(key(other, 0), texture, 4);
        cache.evict(4);
        assert_eq!(cache.entries.len(), 3);
        // Over the byte budget: the least recently used entry goes, never one of composite 5.
        let texture = cache.texture(&gpu.device, (4, 4));
        cache.insert(key(other, 1), texture, 5);
        cache.touch(&key(media, 1), 5);
        cache.evict(5);
        assert_eq!(cache.entries.len(), 3);
        assert!(!cache.entries.contains_key(&key(media, 2)));
        assert!(cache.entries.contains_key(&key(media, 1)));
        assert_eq!(cache.bytes, 3 * 4 * 4 * TEXEL_BYTES);
    }
}
