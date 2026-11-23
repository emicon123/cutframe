//! `convert.wgsl` against `engine::color::convert_pixel` on WARP (render-graph.md §12, "Shader
//! parity"), plus the geometry of the pass: rotation, downscale, coded padding and alpha.
//!
//! The device is always the software adapter (WARP, D3D12) with `TEXTURE_FORMAT_16BIT_NORM`, so
//! every machine and CI compute the same pixels. WARP ships with Windows: a missing adapter fails
//! the tests instead of skipping them. `cargo test -p engine convert_parity -- --nocapture` prints
//! the measured errors.

use app_core::media::Rotation;

use super::convert::{
    AlphaMode, CONVERT_FORMAT, ConvertParams, ConvertPass, SourcePlane, SourceTextures,
    TransferPath,
};
use crate::color::{ColorRange, YuvMatrix, convert_pixel};
use crate::media::frame::{CpuFrame, PlaneBuf, PlaneLayout, SampleDepth};

struct Warp {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pass: ConvertPass,
}

fn warp() -> Warp {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: true,
        compatible_surface: None,
        ..Default::default()
    }))
    .expect("WARP (the D3D12 software adapter) ships with Windows");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("convert_parity"),
        required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .expect("WARP device with TEXTURE_FORMAT_16BIT_NORM");
    let pass = ConvertPass::new(&device);
    Warp {
        device,
        queue,
        pass,
    }
}

/// IEEE half to f32 (normal, subnormal and zero; the pass writes no inf or NaN).
fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * if exponent == 0 {
        mantissa * 2f32.powi(-24)
    } else {
        (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
    }
}

impl Warp {
    /// Converts `source` to `size` and reads the result back: RGBA, rows top to bottom.
    fn convert(
        &self,
        source: &SourceTextures,
        params: &ConvertParams,
        size: (u32, u32),
    ) -> Vec<[f32; 4]> {
        let (width, height) = size;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("convert_parity.target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CONVERT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let row = width * 8;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("convert_parity.readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.pass
            .encode(&self.device, &mut encoder, source, params, &view, size);
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        let submission = self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.expect("map read-back"));
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .expect("GPU poll");
        let mapped = slice.get_mapped_range().expect("mapped read-back");
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for line in mapped.chunks_exact(padded as usize) {
            for px in line[..row as usize].as_chunks::<8>().0 {
                let half = |i: usize| f16_to_f32(u16::from_le_bytes([px[i], px[i + 1]]));
                pixels.push([half(0), half(2), half(4), half(6)]);
            }
        }
        pixels
    }

    fn upload(&self, frame: &CpuFrame) -> SourceTextures {
        SourceTextures::upload(&self.device, &self.queue, frame, None).expect("upload")
    }
}

/// Appends `code` as one sample of `depth`.
fn store(depth: SampleDepth, code: u16, out: &mut Vec<u8>) {
    match depth {
        SampleDepth::U8 => out.push(code as u8),
        SampleDepth::U16 {
            bits,
            msb_aligned: true,
        } => out.extend_from_slice(&(code << (16 - bits)).to_le_bytes()),
        SampleDepth::U16 { .. } => out.extend_from_slice(&code.to_le_bytes()),
    }
}

/// A tight plane of `w x h` texels, each `samples(x, y)` stored at `depth`.
fn plane<const N: usize>(
    depth: SampleDepth,
    (w, h): (u32, u32),
    samples: impl Fn(u32, u32) -> [u16; N],
) -> Option<PlaneBuf> {
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for code in samples(x, y) {
                store(depth, code, &mut bytes);
            }
        }
    }
    Some(PlaneBuf {
        stride: w as usize * N * depth.bytes(),
        bytes,
    })
}

/// A Y'CbCr frame with luma `y(x, y)` and chroma `c(cx, cy)` codes: 4:2:0 semi-planar, or planar
/// with chroma planes of `chroma` size.
fn yuv(
    semi: bool,
    depth: SampleDepth,
    visible: (u32, u32),
    chroma: (u32, u32),
    y: impl Fn(u32, u32) -> u16,
    c: impl Fn(u32, u32) -> (u16, u16),
) -> CpuFrame {
    let luma = plane(depth, visible, |x, row| [y(x, row)]);
    let (layout, planes) = if semi {
        let cbcr = plane(depth, chroma, |x, row| {
            let (u, v) = c(x, row);
            [u, v]
        });
        (PlaneLayout::SemiPlanar, [luma, cbcr, None])
    } else {
        let cb = plane(depth, chroma, |x, row| [c(x, row).0]);
        let cr = plane(depth, chroma, |x, row| [c(x, row).1]);
        let layout = PlaneLayout::Planar3 {
            chroma_w: chroma.0,
            chroma_h: chroma.1,
        };
        (layout, [luma, cb, cr])
    };
    let frame = CpuFrame {
        layout,
        depth,
        visible,
        planes,
    };
    frame.validate().expect("valid test frame");
    frame
}

/// A packed RGBA8 frame from `pixel(x, y)`.
fn rgba8(visible: (u32, u32), pixel: impl Fn(u32, u32) -> [u8; 4]) -> CpuFrame {
    CpuFrame {
        layout: PlaneLayout::Packed,
        depth: SampleDepth::U8,
        visible,
        planes: [
            plane(SampleDepth::U8, visible, |x, y| pixel(x, y).map(u16::from)),
            None,
            None,
        ],
    }
}

fn params(matrix: Option<YuvMatrix>, range: ColorRange, transfer: TransferPath) -> ConvertParams {
    ConvertParams {
        matrix,
        range,
        transfer,
        rotation: Rotation::None,
        alpha: AlphaMode::Opaque,
        display_size: (2, 2),
    }
}

#[track_caller]
fn assert_rgb(got: [f32; 4], expected: [f32; 3], what: &str) {
    for c in 0..3 {
        assert!(
            (got[c] - expected[c]).abs() <= 1.0 / 1023.0,
            "{what}: {got:?} != {expected:?}"
        );
    }
}

/// 10-bit chroma pairs, one band of two chroma rows (four luma rows) each.
const CHROMA_BANDS: [(u16, u16); 11] = [
    (512, 512),
    (64, 512),
    (960, 512),
    (512, 64),
    (512, 960),
    (300, 700),
    (700, 300),
    (128, 128),
    (896, 896),
    (0, 1023),
    (1023, 0),
];

const P010: SampleDepth = SampleDepth::U16 {
    bits: 10,
    msb_aligned: true,
};
const LSB10: SampleDepth = SampleDepth::U16 {
    bits: 10,
    msb_aligned: false,
};

/// The ramp frame, 4:2:0: luma code `x` (10-bit) or `x / 4` (8-bit) across 1024 columns, a chroma
/// band every four rows.
fn ramp(semi: bool, depth: SampleDepth) -> CpuFrame {
    let shift = 10 - depth.bits().min(10);
    let size = (1024, 4 * CHROMA_BANDS.len() as u32);
    yuv(
        semi,
        depth,
        size,
        (size.0 / 2, size.1 / 2),
        |x, _| (x as u16) >> shift,
        |_, cy| {
            let (cb, cr) = CHROMA_BANDS[(cy / 2) as usize];
            (cb >> shift, cr >> shift)
        },
    )
}

#[test]
fn shader_matches_the_reference_on_ramps() {
    let warp = warp();
    let bt2020 = Some(YuvMatrix::Bt2020Ncl);
    let cases = [
        (
            "SDR BT.709 limited",
            params(
                Some(YuvMatrix::Bt709),
                ColorRange::Limited,
                TransferPath::Sdr,
            ),
            1.0,
        ),
        (
            "SDR BT.601 full",
            params(Some(YuvMatrix::Bt601), ColorRange::Full, TransferPath::Sdr),
            1.0,
        ),
        (
            "SDR BT.2020 primaries",
            params(bt2020, ColorRange::Limited, TransferPath::SdrBt2020),
            1.0,
        ),
        (
            "HLG",
            params(bt2020, ColorRange::Limited, TransferPath::Hlg),
            2.0,
        ),
        (
            "PQ 1000 nits",
            params(
                bt2020,
                ColorRange::Limited,
                TransferPath::Pq { peak_nits: 1000 },
            ),
            2.0,
        ),
        (
            "PQ 4000 nits full",
            params(
                bt2020,
                ColorRange::Full,
                TransferPath::Pq { peak_nits: 4000 },
            ),
            2.0,
        ),
    ];
    let frames = [
        ("P010", ramp(true, P010)),
        ("yuv420p10le", ramp(false, LSB10)),
        ("nv12", ramp(true, SampleDepth::U8)),
        ("yuv420p", ramp(false, SampleDepth::U8)),
    ];
    for (frame_name, frame) in &frames {
        let source = warp.upload(frame);
        let bits = frame.depth.bits();
        let shift = 10 - bits.min(10);
        let (w, _) = frame.visible;
        for (case, params, tolerance_lsb) in &cases {
            let out = warp.convert(&source, params, frame.visible);
            let mut max_error = 0.0f32;
            for (band, &(cb, cr)) in CHROMA_BANDS.iter().enumerate() {
                let band = band as u32;
                // Rows whose two vertical chroma taps both lie inside the band.
                for y in [4 * band + 1, 4 * band + 2] {
                    for x in 0..w {
                        let code = [
                            f32::from((x as u16) >> shift),
                            f32::from(cb >> shift),
                            f32::from(cr >> shift),
                        ];
                        let expected = convert_pixel(code, bits, params);
                        let got = out[(y * w + x) as usize];
                        assert_eq!(got[3], 1.0, "{frame_name} {case}: alpha");
                        for c in 0..3 {
                            max_error = max_error.max((got[c] - expected[c]).abs());
                        }
                    }
                }
            }
            eprintln!(
                "{frame_name:12} {case:22} max error {:.3} / 1023",
                max_error * 1023.0
            );
            assert!(
                max_error <= tolerance_lsb / 1023.0,
                "{frame_name} {case}: max error {} / 1023",
                max_error * 1023.0
            );

            if bits == 10 {
                // The grey band of a 10-bit ramp keeps its 10-bit steps: no 8-bit quantisation
                // before the first `Rgba16Float` store.
                let mut levels: Vec<u32> =
                    (0..w).map(|x| out[(w + x) as usize][1].to_bits()).collect();
                levels.sort_unstable();
                levels.dedup();
                eprintln!("{frame_name:12} {case:22} {} distinct levels", levels.len());
                assert!(
                    levels.len() > 256,
                    "{frame_name} {case}: {} levels",
                    levels.len()
                );
            }
        }
    }
}

#[test]
fn quarter_turns_are_uv_swizzles() {
    let warp = warp();
    let (w, h) = (4, 2);
    let colour = |x: u32, y: u32| [(x * 60) as u8, (y * 100) as u8, 30, 255];
    let source = warp.upload(&rgba8((w, h), colour));
    for rotation in [
        Rotation::None,
        Rotation::Cw90,
        Rotation::Cw180,
        Rotation::Cw270,
    ] {
        let size = if rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        };
        let params = ConvertParams {
            rotation,
            display_size: size,
            ..params(None, ColorRange::Full, TransferPath::Sdr)
        };
        let out = warp.convert(&source, &params, size);
        for oy in 0..size.1 {
            for ox in 0..size.0 {
                // Inverse of `RgbImage::rotated`: where the output pixel comes from.
                let (x, y) = match rotation {
                    Rotation::None => (ox, oy),
                    Rotation::Cw90 => (oy, h - 1 - ox),
                    Rotation::Cw180 => (w - 1 - ox, h - 1 - oy),
                    Rotation::Cw270 => (w - 1 - oy, ox),
                };
                let expected = colour(x, y).map(|v| f32::from(v) / 255.0);
                assert_rgb(
                    out[(oy * size.0 + ox) as usize],
                    [expected[0], expected[1], expected[2]],
                    &format!("{rotation:?} at ({ox}, {oy})"),
                );
            }
        }
    }
}

#[test]
fn downscaling_averages_k_by_k_taps() {
    let warp = warp();
    let n = 24;
    let luma = |x: u32, y: u32| ((x * 37 + y * 101) % 256) as u16;
    // 4:4:4, full range, neutral chroma: the output is the luma code / 255.
    let frame = yuv(false, SampleDepth::U8, (n, n), (n, n), luma, |_, _| {
        (128, 128)
    });
    let source = warp.upload(&frame);
    let grey = params(Some(YuvMatrix::Bt709), ColorRange::Full, TransferPath::Sdr);
    // Integer ratios 2, 3 and 4 take 2, 3 and 4 taps per axis at texel centres: a box filter.
    for out_size in [12, 8, 6] {
        let ratio = n / out_size;
        let out = warp.convert(&source, &grey, (out_size, out_size));
        for oy in 0..out_size {
            for ox in 0..out_size {
                let mut sum = 0.0;
                for dy in 0..ratio {
                    for dx in 0..ratio {
                        sum += f32::from(luma(ox * ratio + dx, oy * ratio + dy));
                    }
                }
                let expected = sum / (ratio * ratio) as f32 / 255.0;
                assert_rgb(
                    out[(oy * out_size + ox) as usize],
                    [expected; 3],
                    &format!("ratio {ratio} at ({ox}, {oy})"),
                );
            }
        }
    }
}

/// A texture of `size` whose texel at (x, y) holds the bytes `texel(x, y)`.
fn filled_plane(
    warp: &Warp,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    texel: impl Fn(u32, u32) -> Vec<u8>,
) -> SourcePlane {
    let texture = warp.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("convert_parity.plane"),
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
    });
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend(texel(x, y));
        }
    }
    warp.queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.len() as u32 / height),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    SourcePlane {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        size: (width, height),
    }
}

#[test]
fn coded_padding_is_never_sampled() {
    // Like a hardware NV12 surface of a 1080p frame (1088 coded lines): 8 x 8 visible inside an
    // 8 x 12 luma plane and a 4 x 6 chroma plane, with bright red padding below the picture.
    let warp = warp();
    let luma = filled_plane(&warp, wgpu::TextureFormat::R8Unorm, (8, 12), |_, y| {
        vec![if y < 8 { 16 } else { 235 }]
    });
    let chroma = filled_plane(&warp, wgpu::TextureFormat::Rg8Unorm, (4, 6), |_, y| {
        if y < 4 { vec![128, 128] } else { vec![90, 240] }
    });
    let source = SourceTextures::from_planes(
        PlaneLayout::SemiPlanar,
        SampleDepth::U8,
        (8, 8),
        [Some(luma), Some(chroma), None],
    );
    let black = params(
        Some(YuvMatrix::Bt709),
        ColorRange::Limited,
        TransferPath::Sdr,
    );
    let out = warp.convert(&source, &black, (8, 8));
    for (i, px) in out.iter().enumerate() {
        assert_rgb(*px, [0.0; 3], &format!("pixel {i}"));
    }
}

#[test]
fn images_are_premultiplied() {
    let warp = warp();
    let pixel = |x: u32, _| match x {
        0 => [200, 100, 50, 128],
        1 => [255, 255, 255, 0],
        _ => [10, 20, 30, 255],
    };
    let source = warp.upload(&rgba8((3, 1), pixel));
    let image = ConvertParams {
        alpha: AlphaMode::Straight,
        ..params(None, ColorRange::Full, TransferPath::Sdr)
    };
    let out = warp.convert(&source, &image, (3, 1));
    let a = 128.0 / 255.0;
    let straight = [200.0, 100.0, 50.0].map(|v: f32| v / 255.0 * a);
    assert_rgb(out[0], straight, "half transparent");
    assert!((out[0][3] - a).abs() < 1e-3);
    assert_eq!(out[1], [0.0; 4], "transparent");
    assert_rgb(out[2], [10.0, 20.0, 30.0].map(|v: f32| v / 255.0), "opaque");
    assert_eq!(out[2][3], 1.0);

    // Video ignores the alpha channel.
    let video = params(None, ColorRange::Full, TransferPath::Sdr);
    let out = warp.convert(&source, &video, (3, 1));
    assert!(out.iter().all(|px| px[3] == 1.0));
    assert_rgb(
        out[0],
        [200.0, 100.0, 50.0].map(|v: f32| v / 255.0),
        "opaque video",
    );
}
