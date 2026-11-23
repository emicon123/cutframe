//! The source-convert pass (design `docs/design/render-graph.md` §4.2 step 2, §5.2): one decoded
//! frame to an upright, display-shaped `Rgba16Float` texture of working R'G'B'A.
//!
//! [`ConvertParams`] describes a stream once (it is part of the source-cache key);
//! [`SourceTextures`] holds one frame's planes as textures, uploaded from a [`CpuFrame`] or, for
//! zero-copy hardware frames, created as `Plane0` / `Plane1` views of an NV12 or P010 texture;
//! [`ConvertPass`] draws it. The per-pixel maths is `engine::color::convert_pixel`.

#![allow(
    dead_code,
    reason = "used by the compositor and the readers, tasks 0320 steps 4 and 5"
)]

use app_core::media::{Rotation, VideoStreamInfo};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::color::{self, ColorRange, Eetf, YuvMatrix};
use crate::media::frame::{self, CpuFrame, FrameError, PlaneLayout, SampleDepth};

const SHADER: &str = include_str!("shaders/convert.wgsl");

/// Format of converted source textures (and of every working texture, §5.1).
pub(crate) const CONVERT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Most taps per axis of the downscale filter.
const MAX_TAPS: u32 = 4;

/// How one stream's frames are converted, built once per stream. Part of the source-cache key,
/// so it holds only integers and enums (`Eq + Hash`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConvertParams {
    /// The Y'CbCr matrix; `None` for sources that reach the shader as RGBA.
    pub matrix: Option<YuvMatrix>,
    pub range: ColorRange,
    pub transfer: TransferPath,
    pub rotation: Rotation,
    pub alpha: AlphaMode,
    /// The upright picture size (`VideoStreamInfo::display_size`): the largest convert size.
    pub display_size: (u32, u32),
}

/// What happens after the matrix (§5.2 steps 4 and 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferPath {
    /// R'G'B' are the working values (BT.601 and BT.709 primaries).
    Sdr,
    /// SDR with BT.2020 primaries: linearise, BT.2020 to BT.709, clip, re-encode.
    SdrBt2020,
    /// PQ (and Dolby Vision without a compatible base layer) with source peak `peak_nits`.
    Pq {
        peak_nits: u32,
    },
    Hlg,
}

/// The alpha of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaMode {
    /// Video: alpha is 1 whatever the frame holds.
    Opaque,
    /// Images: straight alpha, premultiplied by the pass.
    Straight,
}

impl ConvertParams {
    /// The parameters of a video stream (opaque).
    pub fn for_video(info: &VideoStreamInfo) -> Self {
        Self::new(info, AlphaMode::Opaque)
    }

    /// The parameters of a still image (straight alpha, premultiplied in the pass).
    pub fn for_image(info: &VideoStreamInfo) -> Self {
        Self::new(info, AlphaMode::Straight)
    }

    fn new(info: &VideoStreamInfo, alpha: AlphaMode) -> Self {
        let rgb = frame::decodes_to_rgb(&info.pixel_format);
        let color = &info.color;
        let matrix = (!rgb).then(|| YuvMatrix::from_tag(color.matrix.as_deref(), info.height));
        let range = ColorRange::from_tag(color.range.as_deref(), &info.pixel_format, rgb);
        let transfer = match info.hdr {
            Some(hdr) => match color::HdrToneMap::for_hdr(&hdr).curve {
                color::HdrCurve::Hlg => TransferPath::Hlg,
                color::HdrCurve::Pq => TransferPath::Pq {
                    peak_nits: color::source_peak_nits(&hdr),
                },
            },
            None if color.primaries.as_deref() == Some("bt2020") => TransferPath::SdrBt2020,
            None => TransferPath::Sdr,
        };
        Self {
            matrix,
            range,
            transfer,
            rotation: info.rotation,
            alpha,
            display_size: info.display_size(),
        }
    }
}

/// The size a source is converted to (§4.2): its on-canvas size in render pixels, capped at the
/// display size, rounded up to even (at least 2x2).
pub(crate) fn convert_size(on_canvas: (u32, u32), display: (u32, u32)) -> (u32, u32) {
    let even = |v: u32| v.max(1).div_ceil(2) * 2;
    if on_canvas.0 >= display.0 || on_canvas.1 >= display.1 {
        (even(display.0), even(display.1))
    } else {
        (even(on_canvas.0), even(on_canvas.1))
    }
}

/// Taps of the downscale filter along one axis: 1 below a ratio of 2, else
/// `min(ceil(ratio), 4)` (§4.2).
fn taps(source_texels: u32, output: u32) -> u32 {
    let ratio = source_texels as f32 / output.max(1) as f32;
    if ratio < 2.0 {
        1
    } else {
        (ratio.ceil() as u32).min(MAX_TAPS)
    }
}

/// The normalised coordinate of the centre of texel `valid - 1` in a plane `size` texels wide:
/// the shader clamps to it, so bilinear taps at the bottom and right edges never read coded
/// padding.
fn last_centre(valid: u32, size: u32) -> f32 {
    (valid.clamp(1, size.max(1)) as f32 - 0.5) / size.max(1) as f32
}

/// One plane as a texture view and the view's size in texels (which may include coded padding).
#[derive(Debug, Clone)]
pub(crate) struct SourcePlane {
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

/// One frame's planes on the GPU, ready for [`ConvertPass::encode`].
#[derive(Debug)]
pub(crate) struct SourceTextures {
    pub layout: PlaneLayout,
    pub depth: SampleDepth,
    /// The picture inside the planes; the rest of plane 0 is coded padding.
    pub visible: (u32, u32),
    pub planes: [Option<SourcePlane>; 3],
    /// Textures created by [`Self::upload`], kept for reuse by the next frame of the same shape.
    owned: Vec<wgpu::Texture>,
}

/// Why a frame cannot be converted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("16-bit planes need the TEXTURE_FORMAT_16BIT_NORM device feature")]
    No16BitNorm,
}

/// The texture format of plane `plane`.
fn plane_format(layout: PlaneLayout, depth: SampleDepth, plane: usize) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as F;
    match (layout.channels(plane), depth) {
        (1, SampleDepth::U8) => F::R8Unorm,
        (2, SampleDepth::U8) => F::Rg8Unorm,
        (_, SampleDepth::U8) => F::Rgba8Unorm,
        (1, SampleDepth::U16 { .. }) => F::R16Unorm,
        (2, SampleDepth::U16 { .. }) => F::Rg16Unorm,
        (_, SampleDepth::U16 { .. }) => F::Rgba16Unorm,
    }
}

impl SourceTextures {
    /// Planes that already are on the GPU, e.g. the `Plane0` / `Plane1` views of a zero-copy
    /// NV12 (`SemiPlanar`, `U8`) or P010 (`SemiPlanar`, `U16 { bits: 10, msb_aligned: true }`)
    /// frame, whose `visible` size is smaller than its coded texture.
    pub fn from_planes(
        layout: PlaneLayout,
        depth: SampleDepth,
        visible: (u32, u32),
        planes: [Option<SourcePlane>; 3],
    ) -> Self {
        Self {
            layout,
            depth,
            visible,
            planes,
            owned: Vec::new(),
        }
    }

    /// Uploads `frame`'s planes, into `reuse`'s textures when they have the same shape.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &CpuFrame,
        reuse: Option<SourceTextures>,
    ) -> Result<Self, ConvertError> {
        frame.validate()?;
        if matches!(frame.depth, SampleDepth::U16 { .. })
            && !device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        {
            return Err(ConvertError::No16BitNorm);
        }
        let count = frame.layout.planes();
        let shape = |index: usize| {
            (
                plane_format(frame.layout, frame.depth, index),
                frame.layout.plane_size(index, frame.visible),
            )
        };
        let reusable = reuse.filter(|old| {
            old.owned.len() == count
                && old.owned.iter().enumerate().all(|(index, texture)| {
                    let (format, (w, h)) = shape(index);
                    texture.format() == format && texture.width() == w && texture.height() == h
                })
        });
        let textures = match reusable {
            Some(old) => old.owned,
            None => (0..count)
                .map(|index| {
                    let (format, (width, height)) = shape(index);
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("convert.plane"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    })
                })
                .collect(),
        };
        let mut planes = [None, None, None];
        for (index, texture) in textures.iter().enumerate() {
            let buf = frame.planes[index]
                .as_ref()
                .ok_or(FrameError::MissingPlane(index))?;
            let (width, height) = frame.layout.plane_size(index, frame.visible);
            queue.write_texture(
                texture.as_image_copy(),
                &buf.bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(buf.stride as u32),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            planes[index] = Some(SourcePlane {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                size: (width, height),
            });
        }
        Ok(Self {
            layout: frame.layout,
            depth: frame.depth,
            visible: frame.visible,
            planes,
            owned: textures,
        })
    }

    fn plane_size(&self, index: usize) -> (u32, u32) {
        self.planes[index]
            .as_ref()
            .map_or(self.visible, |plane| plane.size)
    }
}

/// The shader's `Params` (`shaders/convert.wgsl`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniforms {
    plane_scale: [f32; 4],
    plane_max: [f32; 4],
    out_texel: [f32; 4],
    code: [f32; 4],
    range: [f32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    tone: [f32; 4],
    modes: [u32; 4],
    taps: [u32; 4],
}

impl Uniforms {
    fn new(params: &ConvertParams, source: &SourceTextures, out: (u32, u32)) -> Self {
        let (vw, vh) = (source.visible.0 as f32, source.visible.1 as f32);
        let (lw, lh) = source.plane_size(0);
        let (sx, sy) = source.layout.subsampling(source.visible);
        let (cw, ch) = match source.layout {
            PlaneLayout::Packed => (lw, lh),
            _ => source.plane_size(1),
        };
        let (cx, cy) = match source.layout {
            PlaneLayout::Packed => (vw, vh),
            _ => (vw / sx as f32, vh / sy as f32),
        };
        // Chroma texels that hold picture (the last one of an odd size included).
        let (valid_cw, valid_ch) = match source.layout {
            PlaneLayout::Packed => source.visible,
            _ => (source.visible.0.div_ceil(sx), source.visible.1.div_ceil(sy)),
        };
        let bits = source.depth.bits();
        let rgb = params.matrix.is_none();
        let range = color::RangeCoefficients::new(params.range, bits, rgb);
        let m = color::source_matrix(params.matrix);
        let row = |r: [f32; 3]| [r[0], r[1], r[2], 0.0];
        let eetf = match params.transfer {
            TransferPath::Pq { peak_nits } => Eetf::new(peak_nits as f32),
            _ => Eetf::new(color::HLG_DISPLAY_PEAK_NITS),
        };
        let layout = match source.layout {
            PlaneLayout::Planar3 { .. } => 0,
            PlaneLayout::SemiPlanar => 1,
            PlaneLayout::Packed => 2,
        };
        let transfer = match params.transfer {
            TransferPath::Sdr => 0,
            TransferPath::SdrBt2020 => 1,
            TransferPath::Pq { .. } => 2,
            TransferPath::Hlg => 3,
        };
        let rotation = match params.rotation {
            Rotation::None => 0,
            Rotation::Cw90 => 1,
            Rotation::Cw180 => 2,
            Rotation::Cw270 => 3,
        };
        // Source texels along the output's x and y axes.
        let (along_x, along_y) = if params.rotation.swaps_axes() {
            (source.visible.1, source.visible.0)
        } else {
            source.visible
        };
        let (ow, oh) = (out.0.max(1), out.1.max(1));
        Self {
            plane_scale: [
                vw / lw as f32,
                vh / lh as f32,
                cx / cw as f32,
                cy / ch as f32,
            ],
            plane_max: [
                last_centre(source.visible.0, lw),
                last_centre(source.visible.1, lh),
                last_centre(valid_cw, cw),
                last_centre(valid_ch, ch),
            ],
            out_texel: [1.0 / ow as f32, 1.0 / oh as f32, 0.0, 0.0],
            code: [
                source.depth.code_scale(),
                range.y_offset,
                range.y_scale,
                range.c_offset,
            ],
            range: [range.c_scale, 0.0, 0.0, 0.0],
            m0: row(m[0]),
            m1: row(m[1]),
            m2: row(m[2]),
            tone: [eetf.source_pq, eetf.max_lum, eetf.knee, 0.0],
            modes: [
                layout,
                transfer,
                rotation,
                u32::from(params.alpha == AlphaMode::Straight),
            ],
            taps: [taps(along_x, ow), taps(along_y, oh), 0, 0],
        }
    }
}

/// The convert pipeline. One per device; [`Self::encode`] records one draw per source frame.
#[derive(Debug)]
pub(crate) struct ConvertPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound to the plane slots a layout does not use.
    unused: wgpu::TextureView,
}

impl ConvertPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("convert"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("convert.layout"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2),
                texture(3),
                texture(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("convert.pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("convert.pipeline"),
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
                    format: CONVERT_FORMAT,
                    blend: None,
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
            label: Some("convert.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let unused = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("convert.unused_plane"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            pipeline,
            layout,
            sampler,
            unused,
        }
    }

    /// Records the conversion of `source` into `target`, a [`CONVERT_FORMAT`] view of
    /// `target_size` (normally [`convert_size`]). Upright, display-shaped, premultiplied.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &SourceTextures,
        params: &ConvertParams,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
    ) {
        let uniforms = Uniforms::new(params, source, target_size);
        // One small buffer per draw: several sources are converted in one submission, so a
        // shared buffer written with `Queue::write_buffer` would hold only the last one.
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("convert.uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = |index: usize| {
            source.planes[index]
                .as_ref()
                .map_or(&self.unused, |plane| &plane.view)
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert.bind_group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view(0)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(view(1)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view(2)),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("convert.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use app_core::Rational;
    use app_core::media::{ColorInfo, HdrInfo, HdrTransfer};

    use super::*;

    fn info(pixel_format: &str, color: ColorInfo, hdr: Option<HdrInfo>) -> VideoStreamInfo {
        VideoStreamInfo {
            stream_index: 0,
            codec: "hevc".into(),
            width: 1920,
            height: 1080,
            sample_aspect: Rational::ONE,
            rotation: Rotation::Cw90,
            frame_rate: None,
            pixel_format: pixel_format.into(),
            bit_depth: 10,
            color,
            hdr,
        }
    }

    fn tags(primaries: &str, matrix: &str, range: Option<&str>) -> ColorInfo {
        ColorInfo {
            primaries: Some(primaries.into()),
            transfer: None,
            matrix: Some(matrix.into()),
            range: range.map(Into::into),
        }
    }

    #[test]
    fn params_from_stream_info() {
        let hlg = HdrInfo {
            transfer: HdrTransfer::Hlg,
            mastering_max_nits: None,
            max_cll_nits: None,
            dolby_vision: None,
        };
        let p = ConvertParams::for_video(&info(
            "yuv420p10le",
            tags("bt2020", "bt2020nc", Some("tv")),
            Some(hlg),
        ));
        assert_eq!(p.matrix, Some(YuvMatrix::Bt2020Ncl));
        assert_eq!(p.range, ColorRange::Limited);
        assert_eq!(p.transfer, TransferPath::Hlg);
        assert_eq!((p.rotation, p.display_size), (Rotation::Cw90, (1080, 1920)));
        assert_eq!(p.alpha, AlphaMode::Opaque);

        let pq = HdrInfo {
            transfer: HdrTransfer::DolbyVisionOnly,
            max_cll_nits: Some(2000),
            ..hlg
        };
        let p = ConvertParams::for_video(&info("p010le", ColorInfo::default(), Some(pq)));
        assert_eq!(p.transfer, TransferPath::Pq { peak_nits: 2000 });
        assert_eq!(p.matrix, Some(YuvMatrix::Bt709), "untagged HD");

        let p = ConvertParams::for_video(&info("yuv420p", tags("bt2020", "bt2020nc", None), None));
        assert_eq!(p.transfer, TransferPath::SdrBt2020);
        let p = ConvertParams::for_image(&info("rgb24", ColorInfo::default(), None));
        assert_eq!((p.matrix, p.range), (None, ColorRange::Full));
        assert_eq!(
            (p.transfer, p.alpha),
            (TransferPath::Sdr, AlphaMode::Straight)
        );
        let p = ConvertParams::for_image(&info("yuvj420p", ColorInfo::default(), None));
        assert_eq!(p.range, ColorRange::Full);
    }

    #[test]
    fn convert_size_is_capped_and_even() {
        assert_eq!(convert_size((641, 361), (1920, 1080)), (642, 362));
        assert_eq!(convert_size((3840, 2160), (1920, 1080)), (1920, 1080));
        assert_eq!(convert_size((1920, 1080), (1920, 1080)), (1920, 1080));
        assert_eq!(convert_size((0, 0), (1920, 1080)), (2, 2));
    }

    #[test]
    fn downscale_taps() {
        assert_eq!(taps(1920, 1920), 1);
        assert_eq!(taps(1920, 1000), 1);
        assert_eq!(taps(1920, 960), 2);
        assert_eq!(taps(1920, 700), 3);
        assert_eq!(taps(3840, 640), 4);
        assert_eq!(taps(3840, 100), 4);
    }
}
