//! The export NV12 pass (`engine::export::nv12`, export.md §4.3, §11 "NV12 parity") on WARP,
//! the D3D12 software adapter, so every machine and CI compute the same values; a missing
//! adapter fails the tests instead of skipping them.
//!
//! - Parity: `Nv12Pass` on synthetic `Rgba16Float` canvases (ramps, colour bars, noise, values
//!   out of range, premultiplied alpha) against `reference_nv12`: every code value within 1.
//! - Exact codes: black, white and the 128/255 grey are 16/128/128, 235/128/128, 126/128/128.
//! - The staging ring: order, `RingFull`, pipelined use as in the export loop, the copy into
//!   NV12 and I420 `AVFrame`s with their strides.
//! - The offline renderer's `GpuCanvas` through the pass equals the reference conversion of its
//!   `Rgba8` render within 1 (the canvas is `f16`, the `Rgba8` render is quantised).
//!
//! `cargo test -p engine --test export_nv12 -- --nocapture` prints the measured differences.

use std::sync::{Arc, OnceLock};

use app_core::{Clip, FrameRate, Project, Rgba8, Time};
use engine::export::nv12::{
    Nv12Error, Nv12Frame, Nv12Pass, Nv12Planes, RING_SLOTS, YuvLayout, alloc_video_frame,
    reference_nv12,
};
use engine::render::{
    AdapterMode, CANVAS_FORMAT, GpuContext, OfflineOptions, OfflineRenderer, OutputFormat,
    RenderedFrame,
};

fn warp() -> Arc<GpuContext> {
    static GPU: OnceLock<Arc<GpuContext>> = OnceLock::new();
    Arc::clone(GPU.get_or_init(|| {
        let gpu = GpuContext::new_with(AdapterMode::SoftwareOnly)
            .expect("WARP (the D3D12 software adapter) ships with Windows");
        assert!(gpu.info().software, "not the software adapter");
        Arc::new(gpu)
    }))
}

/// f32 to IEEE half, round to nearest even (normal and subnormal values, no inf or NaN).
fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let abs = value.abs();
    if abs < 2f32.powi(-14) {
        return sign | (abs / 2f32.powi(-24)).round_ties_even() as u16;
    }
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    let mut half = ((exponent as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        half += 1;
    }
    sign | half as u16
}

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

/// A canvas texture holding `pixels` (as `f16`); returns it and the values the GPU sees.
fn canvas(
    gpu: &GpuContext,
    size: (u32, u32),
    pixels: &[[f32; 4]],
) -> (wgpu::Texture, wgpu::TextureView, Vec<[f32; 4]>) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("export_nv12.canvas"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CANVAS_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let halves: Vec<u16> = pixels.iter().flatten().map(|&v| f16_bits(v)).collect();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.0 * 8),
            rows_per_image: Some(size.1),
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    let seen = halves
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p.map(f16_to_f32))
        .collect();
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view, seen)
}

/// The planes of `frame` as tight rows.
fn planes_of(frame: &Nv12Frame<'_>) -> Nv12Planes {
    let (w, h) = frame.size();
    let luma = (0..h).flat_map(|y| frame.luma_row(y).to_vec()).collect();
    let chroma = (0..h.div_ceil(2))
        .flat_map(|j| frame.chroma_row(j).to_vec())
        .collect();
    Nv12Planes {
        size: (w, h),
        luma,
        chroma,
    }
}

/// Largest difference and how many values differ at all.
fn diff(a: &[u8], b: &[u8]) -> (u8, usize) {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).fold((0, 0), |(max, count), (&x, &y)| {
        let d = x.abs_diff(y);
        (max.max(d), count + usize::from(d > 0))
    })
}

/// Converts `pixels` on the GPU and returns the planes.
fn convert(
    gpu: &Arc<GpuContext>,
    size: (u32, u32),
    pixels: &[[f32; 4]],
) -> (Nv12Planes, Vec<[f32; 4]>) {
    let (_texture, view, seen) = canvas(gpu, size, pixels);
    let mut pass = Nv12Pass::new(Arc::clone(gpu), size);
    pass.submit(&view, 7).expect("submit");
    let frame = pass.next_ready(true).expect("map").expect("a frame");
    assert_eq!(frame.index(), 7);
    (planes_of(&frame), seen)
}

/// A deterministic synthetic canvas: horizontal R, G, B and grey ramps, 100 % and 75 % colour
/// bars, a one-pixel checkerboard, values outside `0..=1`, premultiplied semi-transparent
/// colours and pseudo-random noise, in horizontal bands.
fn synthetic(size: (u32, u32)) -> Vec<[f32; 4]> {
    let (w, h) = size;
    let bars: [[f32; 3]; 8] = [
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    let mut seed = 0x2545_f491_u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let band = y * 10 / h;
        for x in 0..w {
            let t = x as f32 / (w - 1) as f32;
            let bar = bars[(x * 8 / w) as usize];
            let rgba = match band {
                0 => [t, 0.0, 0.0, 1.0],
                1 => [0.0, t, 0.0, 1.0],
                2 => [0.0, 0.0, t, 1.0],
                3 => [t, t, t, 1.0],
                4 => [bar[0], bar[1], bar[2], 1.0],
                5 => [0.75 * bar[0], 0.75 * bar[1], 0.75 * bar[2], 1.0],
                6 => {
                    let on = ((x + y) % 2) as f32;
                    [on, 1.0 - on, on * 0.5, 1.0]
                }
                7 => [2.0 * t - 0.5, 1.5 - 2.0 * t, -0.25, 1.0],
                8 => {
                    let a = 0.5;
                    [a * t, a * (1.0 - t), a * 0.8, a]
                }
                _ => [noise(), noise(), noise(), 1.0],
            };
            pixels.push(rgba);
        }
    }
    pixels
}

#[test]
fn the_gpu_pass_matches_the_reference_within_one_code_value() {
    let gpu = warp();
    // 322 is not a multiple of 256: the staging rows are padded.
    for size in [(322, 182), (64, 36), (1920, 1080)] {
        let pixels = synthetic(size);
        let (gpu_planes, seen) = convert(&gpu, size, &pixels);
        let reference = reference_nv12(&seen, size);
        let (luma_max, luma_off) = diff(&gpu_planes.luma, &reference.luma);
        let (chroma_max, chroma_off) = diff(&gpu_planes.chroma, &reference.chroma);
        println!(
            "{size:?}: luma max {luma_max} ({luma_off} of {} differ), chroma max {chroma_max} \
             ({chroma_off} of {} differ)",
            reference.luma.len(),
            reference.chroma.len()
        );
        assert!(luma_max <= 1, "luma differs by {luma_max}");
        assert!(chroma_max <= 1, "chroma differs by {chroma_max}");
    }
}

#[test]
fn black_white_and_grey_are_exact_on_the_gpu() {
    let gpu = warp();
    for (rgb, codes) in [
        ([0.0; 3], [16u8, 128, 128]),
        ([1.0; 3], [235, 128, 128]),
        ([128.0 / 255.0; 3], [126, 128, 128]),
        // Transparent black over black, and values the compositor can produce beyond 0..=1.
        ([-0.1; 3], [16, 128, 128]),
        ([1.3; 3], [235, 128, 128]),
    ] {
        let size = (16, 8);
        let pixels = vec![[rgb[0], rgb[1], rgb[2], 1.0]; 16 * 8];
        let (planes, _) = convert(&gpu, size, &pixels);
        assert!(
            planes.luma.iter().all(|&y| y == codes[0]),
            "{rgb:?}: {:?}",
            &planes.luma[..4]
        );
        assert!(
            planes.chroma.chunks(2).all(|c| c == &codes[1..]),
            "{rgb:?}: {:?}",
            &planes.chroma[..4]
        );
    }
}

fn solid(gpu: &GpuContext, size: (u32, u32), grey: f32) -> (wgpu::Texture, wgpu::TextureView) {
    let pixels = vec![[grey, grey, grey, 1.0]; (size.0 * size.1) as usize];
    let (texture, view, _) = canvas(gpu, size, &pixels);
    (texture, view)
}

fn luma_of(grey: f32) -> u8 {
    reference_nv12(&[[grey, grey, grey, 1.0]], (1, 1)).luma[0]
}

#[test]
fn the_ring_hands_out_frames_in_order_and_refuses_a_fourth() {
    let gpu = warp();
    let size = (64, 32);
    let greys = [0.1, 0.4, 0.7, 0.9];
    let canvases: Vec<_> = greys.iter().map(|&g| solid(&gpu, size, g)).collect();
    let mut pass = Nv12Pass::new(Arc::clone(&gpu), size);
    assert!(pass.next_ready(true).unwrap().is_none(), "empty ring");
    for (n, (_, view)) in canvases.iter().take(RING_SLOTS).enumerate() {
        pass.submit(view, n as u64).unwrap();
    }
    assert!(pass.is_full());
    assert_eq!(pass.submit(&canvases[3].1, 3), Err(Nv12Error::RingFull));
    for (n, &grey) in greys.iter().enumerate().take(RING_SLOTS) {
        let frame = pass.next_ready(true).unwrap().expect("a frame");
        assert_eq!(frame.index(), n as u64);
        assert!(frame.luma_row(5).iter().all(|&y| y == luma_of(grey)));
    }
    assert_eq!(pass.in_flight(), 0);
    assert!(pass.next_ready(true).unwrap().is_none());
}

#[test]
fn a_pipelined_loop_copies_every_frame_into_avframes() {
    let gpu = warp();
    let size = (96, 54);
    let greys: Vec<f32> = (0..10).map(|n| n as f32 / 10.0).collect();
    let canvases: Vec<_> = greys.iter().map(|&g| solid(&gpu, size, g)).collect();
    let mut pass = Nv12Pass::new(Arc::clone(&gpu), size);
    let mut nv12 = alloc_video_frame(size, YuvLayout::Nv12).unwrap();
    let mut i420 = alloc_video_frame(size, YuvLayout::Yuv420p).unwrap();
    let mut seen = Vec::new();
    let mut check = |frame: Nv12Frame<'_>| {
        frame.copy_to_avframe(&mut nv12).unwrap();
        frame.copy_to_avframe(&mut i420).unwrap();
        let expected = luma_of(greys[frame.index() as usize]);
        for picture in [&nv12, &i420] {
            for (plane, value, rows, bytes) in [
                (0, expected, size.1, size.0),
                (
                    1,
                    128,
                    size.1 / 2,
                    if picture.format == nv12.format {
                        size.0
                    } else {
                        size.0 / 2
                    },
                ),
            ] {
                let stride = picture.linesize[plane] as usize;
                for row in 0..rows as usize {
                    // SAFETY: an allocated frame of this size and format has `rows` rows of
                    // `linesize[plane] >= bytes` bytes in `plane`.
                    let line = unsafe {
                        std::slice::from_raw_parts(
                            picture.data[plane].add(row * stride),
                            bytes as usize,
                        )
                    };
                    assert!(line.iter().all(|&v| v == value), "plane {plane} row {row}");
                }
            }
        }
        seen.push(frame.index());
    };
    // The export loop: wait only when the ring is full, take whatever is ready after a submit.
    for (n, (_, view)) in canvases.iter().enumerate() {
        if pass.is_full() {
            check(pass.next_ready(true).unwrap().expect("a frame"));
        }
        pass.submit(view, n as u64).unwrap();
        while let Some(frame) = pass.next_ready(false).unwrap() {
            check(frame);
        }
    }
    while let Some(frame) = pass.next_ready(true).unwrap() {
        check(frame);
    }
    assert_eq!(seen, (0..10).collect::<Vec<u64>>());
}

#[test]
fn a_mismatched_avframe_is_refused() {
    let gpu = warp();
    let (_texture, view) = solid(&gpu, (32, 16), 0.5);
    let mut pass = Nv12Pass::new(Arc::clone(&gpu), (32, 16));
    pass.submit(&view, 0).unwrap();
    let frame = pass.next_ready(true).unwrap().unwrap();
    let mut wrong_size = alloc_video_frame((64, 16), YuvLayout::Nv12).unwrap();
    assert!(matches!(
        frame.copy_to_avframe(&mut wrong_size),
        Err(Nv12Error::Frame(_))
    ));
}

#[test]
fn the_offline_canvas_converts_like_its_rgba8_render() {
    let gpu = warp();
    let mut project = Project::new("nv12");
    project.settings.width = 160;
    project.settings.height = 90;
    project.settings.frame_rate = FrameRate::FPS_30;
    let second = Time(app_core::FLICKS_PER_SECOND);
    let mut overlay = Clip::color("Overlay", Time::ZERO, second, Rgba8::rgb(0xe0, 0x30, 0x50));
    if let Some(visual) = overlay.visual.as_mut() {
        visual.opacity = app_core::Animated::constant(0.6);
    }
    project.tracks[0].clips = vec![Clip::color(
        "Base",
        Time::ZERO,
        second,
        Rgba8::rgb(0x20, 0xa0, 0x60),
    )];
    let mut track = app_core::Track::new(app_core::TrackKind::Video, "Overlay");
    track.clips = vec![overlay];
    project.tracks.insert(0, track);
    project.validate().expect("valid");

    let size = (160, 90);
    let mut renderer = OfflineRenderer::new(Arc::clone(&gpu), OfflineOptions::default());
    let rgba: Vec<[f32; 4]> = match renderer.render_frame(&project, 3, size, OutputFormat::Rgba8) {
        Ok(RenderedFrame::Rgba8(bytes)) => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p.map(|v| f32::from(v) / 255.0))
            .collect(),
        other => panic!("{other:?}"),
    };
    let reference = reference_nv12(&rgba, size);
    let mut pass = Nv12Pass::new(Arc::clone(&gpu), size);
    match renderer.render_frame(&project, 3, size, OutputFormat::GpuCanvas) {
        Ok(RenderedFrame::Gpu(view)) => pass.submit(view, 3).unwrap(),
        other => panic!("{other:?}"),
    }
    let frame = pass.next_ready(true).unwrap().unwrap();
    let planes = planes_of(&frame);
    let (luma_max, _) = diff(&planes.luma, &reference.luma);
    let (chroma_max, _) = diff(&planes.chroma, &reference.chroma);
    println!("offline canvas vs Rgba8 reference: luma {luma_max}, chroma {chroma_max}");
    assert!(
        luma_max <= 1 && chroma_max <= 1,
        "luma {luma_max}, chroma {chroma_max}"
    );
}
