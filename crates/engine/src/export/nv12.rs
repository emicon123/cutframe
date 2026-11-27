//! GPU → encoder hand-off of export (export.md §4.3): the `Rgba16Float` canvas to BT.709
//! limited-range NV12 on the GPU, read back through a ring of [`RING_SLOTS`] staging buffers,
//! then copied into encoder frames as NV12 or I420 (`yuv420p`, OpenH264).
//!
//! - **Conversion** (`render/shaders/nv12.wgsl`): the premultiplied canvas over black, R'G'B'
//!   clamped to `0..=1`; `Y' = 0.2126 R' + 0.7152 G' + 0.0722 B'` as code `16 + 219 Y'`,
//!   `Cb = (B' − Y') / 1.8556`, `Cr = (R' − Y') / 1.5748` as `128 + 224 C`, rounded to the nearest
//!   code value, no dithering.
//! - **Chroma siting: left.** Chroma sample `(i, j)` filters R'G'B' with weights 1/4, 1/2, 1/4 on
//!   luma columns `2i − 1, 2i, 2i + 1` and 1/2, 1/2 on rows `2j, 2j + 1`, clamped at the edges.
//! - **Ring.** [`Nv12Pass::submit`] records both passes and the copies into a free slot, submits
//!   them after the render graph's submission (queue order protects the canvas) and starts the
//!   map. [`Nv12Pass::next_ready`] hands out the oldest frame once it is mapped, so the GPU work
//!   of frame `n + 1` overlaps the CPU copy of frame `n`. Dropping the [`Nv12Frame`] unmaps the
//!   slot.
//!
//! [`reference_nv12`] is the CPU definition of the pass (built on [`YuvMatrix::Bt709`]); the WARP
//! parity test (`tests/export_nv12.rs`) holds the shader to it within one code value.
//!
//! Threading: everything here runs on the export thread. The pass owns its textures and staging
//! buffers on the shared device (ADR-0004 §6); nothing is shared with the preview but the queue.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bytemuck::{Pod, Zeroable};
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

use crate::color::YuvMatrix;
use crate::render::GpuContext;

const SHADER: &str = include_str!("../render/shaders/nv12.wgsl");

/// Staging buffers in the read-back ring (export.md §4.3).
pub const RING_SLOTS: usize = 3;

const LUMA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const CHROMA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;

/// Plane layout of an encoder frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YuvLayout {
    /// Y' rows, then interleaved CbCr rows (`AV_PIX_FMT_NV12`): every encoder but OpenH264.
    Nv12,
    /// Y', Cb and Cr planes (`AV_PIX_FMT_YUV420P`, I420): OpenH264.
    Yuv420p,
}

impl YuvLayout {
    /// The FFmpeg pixel format.
    pub fn pixel_format(self) -> ffi::AVPixelFormat {
        match self {
            Self::Nv12 => ffi::AV_PIX_FMT_NV12,
            Self::Yuv420p => ffi::AV_PIX_FMT_YUV420P,
        }
    }

    /// The layout of FFmpeg pixel format `format`, if it is one of the two.
    pub fn from_pixel_format(format: ffi::AVPixelFormat) -> Option<Self> {
        match format {
            ffi::AV_PIX_FMT_NV12 => Some(Self::Nv12),
            ffi::AV_PIX_FMT_YUV420P => Some(Self::Yuv420p),
            _ => None,
        }
    }
}

/// Why the NV12 pass or a copy out of it failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Nv12Error {
    #[error("all {RING_SLOTS} staging slots hold frames that were not read out yet")]
    RingFull,
    #[error("waiting for the GPU failed: {0}")]
    Poll(String),
    #[error("mapping a staging buffer failed")]
    Map,
    #[error("reading a mapped staging buffer failed: {0}")]
    Range(String),
    #[error("the encoder frame does not fit: {0}")]
    Frame(String),
}

/// The geometry of one frame size: plane sizes and the padded rows of a staging buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Geometry {
    size: (u32, u32),
    /// `ceil(w / 2) x ceil(h / 2)`.
    chroma: (u32, u32),
    /// Bytes per Y' row in a staging buffer (`w`, padded to 256).
    luma_row: u32,
    /// Bytes per CbCr row in a staging buffer (`2 ceil(w / 2)`, padded to 256).
    chroma_row: u32,
}

impl Geometry {
    fn new(size: (u32, u32)) -> Self {
        let size = (size.0.max(1), size.1.max(1));
        let chroma = (size.0.div_ceil(2), size.1.div_ceil(2));
        let pad = |bytes: u32| {
            bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
        };
        Self {
            size,
            chroma,
            luma_row: pad(size.0),
            chroma_row: pad(chroma.0 * 2),
        }
    }

    /// Where the CbCr rows start in a staging buffer (a multiple of 256).
    fn chroma_offset(&self) -> u64 {
        u64::from(self.luma_row) * u64::from(self.size.1)
    }

    fn buffer_size(&self) -> u64 {
        self.chroma_offset() + u64::from(self.chroma_row) * u64::from(self.chroma.1)
    }
}

/// The shader's `Params`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
}

const MAP_PENDING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;

#[derive(Debug)]
enum SlotState {
    Free,
    /// Submitted as frame `frame`, the `seq`-th submission of this pass; mapping started.
    Pending {
        frame: u64,
        seq: u64,
        submission: wgpu::SubmissionIndex,
    },
}

#[derive(Debug)]
struct Slot {
    buffer: wgpu::Buffer,
    state: SlotState,
    /// Written by the `map_async` callback (`MAP_*`).
    mapped: Arc<AtomicU8>,
}

/// The NV12 pass of one output size with its staging ring (review F48: every target is created
/// once and reused). Create one per export on the export thread.
#[derive(Debug)]
pub struct Nv12Pass {
    gpu: Arc<GpuContext>,
    geometry: Geometry,
    luma_texture: wgpu::Texture,
    luma_view: wgpu::TextureView,
    chroma_texture: wgpu::Texture,
    chroma_view: wgpu::TextureView,
    luma_pipeline: wgpu::RenderPipeline,
    chroma_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// The bind group of the last canvas view, rebuilt when the canvas changes.
    bound: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    slots: [Slot; RING_SLOTS],
    next_seq: u64,
}

impl Nv12Pass {
    /// The pass for frames of `size` (the export size; `w` and `h` should be even, as every
    /// export size is, export.md §4.1).
    pub fn new(gpu: Arc<GpuContext>, size: (u32, u32)) -> Self {
        let device = &gpu.device;
        let geometry = Geometry::new(size);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Uniforms>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12.pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let luma_pipeline = pipeline("fs_luma", LUMA_FORMAT);
        let chroma_pipeline = pipeline("fs_chroma", CHROMA_FORMAT);

        let target = |label, (width, height), format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let luma_texture = target("nv12.luma", geometry.size, LUMA_FORMAT);
        let luma_view = luma_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let chroma_texture = target("nv12.chroma", geometry.chroma, CHROMA_FORMAT);
        let chroma_view = chroma_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let matrix = YuvMatrix::Bt709.from_rgb();
        let row = |r: [f32; 3]| [r[0], r[1], r[2], 0.0];
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(
            &uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                m0: row(matrix[0]),
                m1: row(matrix[1]),
                m2: row(matrix[2]),
            }),
        );

        let slots = std::array::from_fn(|i| Slot {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(["nv12.staging0", "nv12.staging1", "nv12.staging2"][i]),
                size: geometry.buffer_size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            state: SlotState::Free,
            mapped: Arc::new(AtomicU8::new(MAP_PENDING)),
        });
        Self {
            luma_texture,
            luma_view,
            chroma_texture,
            chroma_view,
            luma_pipeline,
            chroma_pipeline,
            layout,
            uniforms,
            bound: None,
            slots,
            next_seq: 0,
            geometry,
            gpu,
        }
    }

    /// The frame size (`w x h` luma samples).
    pub fn size(&self) -> (u32, u32) {
        self.geometry.size
    }

    /// Frames submitted and not yet handed out by [`Self::next_ready`].
    pub fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s.state, SlotState::Pending { .. }))
            .count()
    }

    /// Every slot holds a frame: [`Self::next_ready`] (with `wait`) must free one before the
    /// next [`Self::submit`].
    pub fn is_full(&self) -> bool {
        self.in_flight() == RING_SLOTS
    }

    /// Converts `canvas` (a [`crate::render::CANVAS_FORMAT`] view of [`Self::size`], e.g.
    /// `RenderedFrame::Gpu`) as frame `frame` into a free staging slot: records the luma and
    /// chroma passes and the two copies, submits them and starts mapping the slot. Returns at
    /// once; the canvas may be rendered again right after (queue order).
    pub fn submit(&mut self, canvas: &wgpu::TextureView, frame: u64) -> Result<(), Nv12Error> {
        let Some(index) = self
            .slots
            .iter()
            .position(|s| matches!(s.state, SlotState::Free))
        else {
            return Err(Nv12Error::RingFull);
        };
        let device = &self.gpu.device;
        if self.bound.as_ref().is_none_or(|(view, _)| view != canvas) {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nv12.bind_group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(canvas),
                    },
                ],
            });
            self.bound = Some((canvas.clone(), bind_group));
        }
        let Some((_, bind_group)) = &self.bound else {
            return Err(Nv12Error::Map);
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nv12"),
        });
        for (view, pipeline) in [
            (&self.luma_view, &self.luma_pipeline),
            (&self.chroma_view, &self.chroma_pipeline),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nv12.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let geometry = self.geometry;
        let slot = &mut self.slots[index];
        let copy = |encoder: &mut wgpu::CommandEncoder,
                    texture: &wgpu::Texture,
                    offset: u64,
                    row: u32,
                    (width, height): (u32, u32)| {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &slot.buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        };
        copy(
            &mut encoder,
            &self.luma_texture,
            0,
            geometry.luma_row,
            geometry.size,
        );
        copy(
            &mut encoder,
            &self.chroma_texture,
            geometry.chroma_offset(),
            geometry.chroma_row,
            geometry.chroma,
        );
        let submission = self.gpu.queue.submit([encoder.finish()]);
        slot.mapped.store(MAP_PENDING, Ordering::Release);
        let mapped = Arc::clone(&slot.mapped);
        slot.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let state = if result.is_ok() { MAP_DONE } else { MAP_FAILED };
                mapped.store(state, Ordering::Release);
            });
        slot.state = SlotState::Pending {
            frame,
            seq: self.next_seq,
            submission,
        };
        self.next_seq += 1;
        Ok(())
    }

    /// The oldest submitted frame once its slot is mapped. With `wait`, blocks until the GPU
    /// finished it; without, returns `Ok(None)` while it is still in flight. `Ok(None)` too when
    /// nothing is in flight. Frames come out in submission order; dropping the returned frame
    /// frees its slot.
    pub fn next_ready(&mut self, wait: bool) -> Result<Option<Nv12Frame<'_>>, Nv12Error> {
        let oldest = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.state {
                SlotState::Pending { seq, .. } => Some((*seq, i)),
                SlotState::Free => None,
            })
            .min()
            .map(|(_, i)| i);
        let Some(index) = oldest else {
            return Ok(None);
        };
        let device = &self.gpu.device;
        let slot = &mut self.slots[index];
        let SlotState::Pending {
            frame, submission, ..
        } = &slot.state
        else {
            return Ok(None);
        };
        let frame = *frame;
        if slot.mapped.load(Ordering::Acquire) == MAP_PENDING {
            let poll = if wait {
                wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: None,
                }
            } else {
                wgpu::PollType::Poll
            };
            device
                .poll(poll)
                .map_err(|err| Nv12Error::Poll(err.to_string()))?;
        }
        match slot.mapped.load(Ordering::Acquire) {
            MAP_DONE => {}
            MAP_PENDING if !wait => return Ok(None),
            _ => {
                // A failed map leaves the buffer unmapped; the slot can be used again.
                slot.state = SlotState::Free;
                return Err(Nv12Error::Map);
            }
        }
        let view = match slot.buffer.get_mapped_range(..) {
            Ok(view) => view,
            Err(err) => {
                slot.buffer.unmap();
                slot.state = SlotState::Free;
                return Err(Nv12Error::Range(err.to_string()));
            }
        };
        Ok(Some(Nv12Frame {
            frame,
            geometry: self.geometry,
            view: Some(view),
            buffer: &slot.buffer,
            state: &mut slot.state,
        }))
    }
}

/// A mapped staging slot: the NV12 planes of one frame. Dropping it unmaps the slot and frees
/// it for the next [`Nv12Pass::submit`].
#[derive(Debug)]
pub struct Nv12Frame<'a> {
    frame: u64,
    geometry: Geometry,
    view: Option<wgpu::BufferView>,
    buffer: &'a wgpu::Buffer,
    state: &'a mut SlotState,
}

impl Nv12Frame<'_> {
    /// The frame number given to [`Nv12Pass::submit`].
    pub fn index(&self) -> u64 {
        self.frame
    }

    /// `w x h` luma samples.
    pub fn size(&self) -> (u32, u32) {
        self.geometry.size
    }

    fn bytes(&self) -> &[u8] {
        self.view.as_deref().unwrap_or(&[])
    }

    /// Y' row `y` (`w` code values).
    pub fn luma_row(&self, y: u32) -> &[u8] {
        let g = &self.geometry;
        let start = y as usize * g.luma_row as usize;
        &self.bytes()[start..start + g.size.0 as usize]
    }

    /// CbCr row `j` (`ceil(w / 2)` interleaved Cb, Cr pairs).
    pub fn chroma_row(&self, j: u32) -> &[u8] {
        let g = &self.geometry;
        let start = g.chroma_offset() as usize + j as usize * g.chroma_row as usize;
        &self.bytes()[start..start + 2 * g.chroma.0 as usize]
    }

    /// Copies the planes into `planes` as `layout`: NV12 takes `[Y', CbCr]`, I420 takes
    /// `[Y', Cb, Cr]`; `strides` are the bytes per row of each plane (at least the row width).
    pub fn copy_to_planes(
        &self,
        layout: YuvLayout,
        planes: &mut [&mut [u8]],
        strides: &[usize],
    ) -> Result<(), Nv12Error> {
        let g = self.geometry;
        let (w, h) = (g.size.0 as usize, g.size.1 as usize);
        let (cw, ch) = (g.chroma.0 as usize, g.chroma.1 as usize);
        let shapes: &[(usize, usize)] = match layout {
            YuvLayout::Nv12 => &[(w, h), (2 * cw, ch)],
            YuvLayout::Yuv420p => &[(w, h), (cw, ch), (cw, ch)],
        };
        for (plane, &(row, rows)) in shapes.iter().enumerate() {
            let fits = planes
                .get(plane)
                .zip(strides.get(plane))
                .is_some_and(|(p, &s)| {
                    s >= row
                        && rows
                            .checked_sub(1)
                            .is_none_or(|last| p.len() >= last * s + row)
                });
            if !fits {
                return Err(Nv12Error::Frame(format!(
                    "plane {plane} is smaller than {rows} rows of {row} bytes"
                )));
            }
        }
        for y in 0..h {
            let s = strides[0];
            planes[0][y * s..y * s + w].copy_from_slice(self.luma_row(y as u32));
        }
        match layout {
            YuvLayout::Nv12 => {
                let s = strides[1];
                for j in 0..ch {
                    planes[1][j * s..j * s + 2 * cw].copy_from_slice(self.chroma_row(j as u32));
                }
            }
            YuvLayout::Yuv420p => {
                let (su, sv) = (strides[1], strides[2]);
                let (u, v) = planes[1..].split_at_mut(1);
                for j in 0..ch {
                    let row = self.chroma_row(j as u32);
                    let u = &mut u[0][j * su..j * su + cw];
                    let v = &mut v[0][j * sv..j * sv + cw];
                    for (i, pair) in row.as_chunks::<2>().0.iter().enumerate() {
                        u[i] = pair[0];
                        v[i] = pair[1];
                    }
                }
            }
        }
        Ok(())
    }

    /// Copies the planes into `frame`, an allocated `nv12` or `yuv420p` frame of this size (see
    /// [`alloc_video_frame`]), making it writable first (an encoder may still hold a reference
    /// to its buffers). The frame's other fields (`pts`, colour tags) are left as they are.
    pub fn copy_to_avframe(&self, frame: &mut AVFrame) -> Result<(), Nv12Error> {
        let (w, h) = self.geometry.size;
        let (cw, ch) = self.geometry.chroma;
        let layout = YuvLayout::from_pixel_format(frame.format).ok_or_else(|| {
            Nv12Error::Frame(format!(
                "pixel format {} is not nv12 or yuv420p",
                frame.format
            ))
        })?;
        if (frame.width, frame.height) != (w as i32, h as i32) {
            return Err(Nv12Error::Frame(format!(
                "frame is {}x{}, the pass {w}x{h}",
                frame.width, frame.height
            )));
        }
        frame
            .make_writable()
            .map_err(|err| Nv12Error::Frame(err.to_string()))?;
        let rows = match layout {
            YuvLayout::Nv12 => [(w, h), (2 * cw, ch), (0, 0)],
            YuvLayout::Yuv420p => [(w, h), (cw, ch), (cw, ch)],
        };
        let count = if layout == YuvLayout::Nv12 { 2 } else { 3 };
        let mut strides = [0usize; 3];
        let mut lens = [0usize; 3];
        for p in 0..count {
            let stride = usize::try_from(frame.linesize[p])
                .map_err(|_| Nv12Error::Frame(format!("plane {p} has a negative stride")))?;
            if frame.data[p].is_null() || stride < rows[p].0 as usize {
                return Err(Nv12Error::Frame(format!("plane {p} is not allocated")));
            }
            strides[p] = stride;
            lens[p] = (rows[p].1 as usize - 1) * stride + rows[p].0 as usize;
        }
        let data = *frame.data_mut();
        let plane = |p: usize| -> &mut [u8] {
            if p >= count {
                return &mut [];
            }
            // SAFETY: the frame is allocated and writable (checked above) with `linesize[p]`-byte
            // rows; `av_frame_get_buffer` allocates at least `rows[p].1` rows of `linesize[p]`
            // bytes per plane of this format and size, so `lens[p]` bytes from `data[p]` are
            // inside the plane buffer, and the planes do not overlap. The slices live until the
            // copy below returns, while `frame` is borrowed mutably.
            unsafe { std::slice::from_raw_parts_mut(data[p], lens[p]) }
        };
        let mut planes = [plane(0), plane(1), plane(2)];
        self.copy_to_planes(layout, &mut planes, &strides[..count])
    }
}

impl Drop for Nv12Frame<'_> {
    fn drop(&mut self) {
        // Every view must be gone before `unmap`.
        drop(self.view.take());
        self.buffer.unmap();
        *self.state = SlotState::Free;
    }
}

/// An allocated `layout` frame of `size` for an encoder (`av_frame_get_buffer`). Allocate one
/// per export and reuse it: [`Nv12Frame::copy_to_avframe`] makes it writable again.
pub fn alloc_video_frame(size: (u32, u32), layout: YuvLayout) -> Result<AVFrame, Nv12Error> {
    let mut frame = AVFrame::new();
    frame.set_format(layout.pixel_format());
    frame.set_width(i32::try_from(size.0).map_err(|_| Nv12Error::Frame("width".into()))?);
    frame.set_height(i32::try_from(size.1).map_err(|_| Nv12Error::Frame("height".into()))?);
    frame
        .alloc_buffer()
        .map_err(|err| Nv12Error::Frame(err.to_string()))?;
    Ok(frame)
}

/// NV12 planes on the CPU: tight Y' rows and tight interleaved CbCr rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nv12Planes {
    pub size: (u32, u32),
    /// `w x h` Y' code values.
    pub luma: Vec<u8>,
    /// `ceil(h / 2)` rows of `ceil(w / 2)` (Cb, Cr) pairs.
    pub chroma: Vec<u8>,
}

/// `floor(value + 0.5)` as a code value (the shader's rounding).
fn quantize(value: f32) -> u8 {
    (value + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// Y' code of working R'G'B' `rgb` (already clamped to `0..=1`).
fn luma_code(m: &[[f32; 3]; 3], rgb: [f32; 3]) -> u8 {
    let y = m[0][0] * rgb[0] + m[0][1] * rgb[1] + m[0][2] * rgb[2];
    quantize(16.0 + 219.0 * y)
}

/// The CPU reference of the NV12 pass (the definition `nv12.wgsl` is tested against):
/// `rgba` holds the canvas values (premultiplied R'G'B'A, rows top to bottom, `size.0 x size.1`),
/// shown over black. Same conversion, chroma filter and rounding as the shader.
pub fn reference_nv12(rgba: &[[f32; 4]], size: (u32, u32)) -> Nv12Planes {
    let g = Geometry::new(size);
    let (w, h) = (g.size.0 as usize, g.size.1 as usize);
    let (cw, ch) = (g.chroma.0 as usize, g.chroma.1 as usize);
    assert!(
        rgba.len() >= w * h,
        "the canvas has fewer than w x h pixels"
    );
    let m = YuvMatrix::Bt709.from_rgb();
    let at = |x: isize, y: isize| -> [f32; 3] {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        let p = rgba[y * w + x];
        [p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0))
    };
    let mut luma = Vec::with_capacity(w * h);
    for y in 0..h as isize {
        for x in 0..w as isize {
            luma.push(luma_code(&m, at(x, y)));
        }
    }
    let row_filtered = |x: isize, y: isize| -> [f32; 3] {
        let (a, b, c) = (at(x - 1, y), at(x, y), at(x + 1, y));
        std::array::from_fn(|k| 0.25 * a[k] + 0.5 * b[k] + 0.25 * c[k])
    };
    let mut chroma = Vec::with_capacity(2 * cw * ch);
    for j in 0..ch as isize {
        for i in 0..cw as isize {
            let (top, bottom) = (row_filtered(2 * i, 2 * j), row_filtered(2 * i, 2 * j + 1));
            let rgb: [f32; 3] = std::array::from_fn(|k| 0.5 * top[k] + 0.5 * bottom[k]);
            let cb = m[1][0] * rgb[0] + m[1][1] * rgb[1] + m[1][2] * rgb[2];
            let cr = m[2][0] * rgb[0] + m[2][1] * rgb[1] + m[2][2] * rgb[2];
            chroma.push(quantize(128.0 + 224.0 * cb));
            chroma.push(quantize(128.0 + 224.0 * cr));
        }
    }
    Nv12Planes {
        size: g.size,
        luma,
        chroma,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(rgb: [f32; 3], size: (u32, u32)) -> Nv12Planes {
        let pixels = vec![[rgb[0], rgb[1], rgb[2], 1.0]; (size.0 * size.1) as usize];
        reference_nv12(&pixels, size)
    }

    #[test]
    fn black_white_and_mid_grey_are_exact() {
        for (rgb, codes) in [
            ([0.0; 3], [16, 128, 128]),
            ([1.0; 3], [235, 128, 128]),
            ([128.0 / 255.0; 3], [126, 128, 128]),
            // Out of range values are clamped first.
            ([-0.5, 2.0, -1.0], [173, 42, 26]),
        ] {
            let planes = flat(rgb, (4, 2));
            assert!(planes.luma.iter().all(|&y| y == codes[0]), "{rgb:?}");
            assert_eq!(&planes.chroma[..2], &codes[1..], "{rgb:?}");
        }
    }

    #[test]
    fn primaries_match_bt709_limited() {
        // BT.709 limited 8-bit codes of the primaries (ITU-R BT.709 table, rounded).
        for (rgb, codes) in [
            ([1.0, 0.0, 0.0], [63, 102, 240]),
            ([0.0, 1.0, 0.0], [173, 42, 26]),
            ([0.0, 0.0, 1.0], [32, 240, 118]),
        ] {
            let planes = flat(rgb, (2, 2));
            assert_eq!(
                [planes.luma[0], planes.chroma[0], planes.chroma[1]],
                codes,
                "{rgb:?}"
            );
        }
    }

    #[test]
    fn chroma_is_left_sited() {
        // A red column at x = 2 in a black 8 x 2 frame: chroma sample 1 (taps 1, 2, 3) has it
        // in its centre tap (weight 1/2); samples 0 (taps -1, 0, 1) and 2 (taps 3, 4, 5) miss it.
        let mut pixels = vec![[0.0, 0.0, 0.0, 1.0]; 16];
        for y in 0..2 {
            pixels[y * 8 + 2] = [1.0, 0.0, 0.0, 1.0];
        }
        let planes = reference_nv12(&pixels, (8, 2));
        let cr: Vec<u8> = planes.chroma.chunks(2).map(|c| c[1]).collect();
        // Cr of half red: 128 + 224 * 0.5 * 0.5 = 184.
        assert_eq!(cr, vec![128, 184, 128, 128]);
        // A red pixel at x = 3 is shared by samples 1 and 2 (weight 1/4 each).
        let mut pixels = vec![[0.0, 0.0, 0.0, 1.0]; 16];
        for y in 0..2 {
            pixels[y * 8 + 3] = [1.0, 0.0, 0.0, 1.0];
        }
        let planes = reference_nv12(&pixels, (8, 2));
        let cr: Vec<u8> = planes.chroma.chunks(2).map(|c| c[1]).collect();
        assert_eq!(cr, vec![128, 156, 156, 128]);
    }

    #[test]
    fn staging_rows_are_padded_and_aligned() {
        let g = Geometry::new((1920, 1080));
        assert_eq!((g.luma_row, g.chroma_row), (2048, 2048));
        assert_eq!(g.chroma, (960, 540));
        assert_eq!(g.chroma_offset() % 256, 0);
        let g = Geometry::new((640, 360));
        assert_eq!((g.luma_row, g.chroma_row), (768, 768));
        assert_eq!(g.buffer_size(), 768 * 360 + 768 * 180);
        let odd = Geometry::new((5, 3));
        assert_eq!(odd.chroma, (3, 2));
    }
}
