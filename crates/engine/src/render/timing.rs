//! Timing of the render graph against the budget of render-graph.md §13. Ignored: it needs a
//! hardware GPU, a release build and (for the convert and decode rows) a 4K HEVC Main 10 clip:
//!
//! ```text
//! CUTFRAME_GPU=integrated CUTFRAME_TIMING_CLIP=C:\clips\hevc-4k60-main10-hlg.mp4 \
//!   cargo test -p engine --release --lib render::timing -- --ignored --nocapture --test-threads 1
//! ```
//!
//! GPU times come from timestamp queries written into the encoder around the measured work
//! (`TIMESTAMP_QUERY_INSIDE_ENCODERS`); adapters without them report wall time around submit and
//! wait instead (the output says which).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::media::{MediaId, MediaPath, Rotation};
use app_core::{ClipId, MediaAsset, Project, Rgba8, Time};

use super::GpuContext;
use super::compositor::{Compositor, CompositorConfig, create_canvas, gpu_source};
use super::convert::{AlphaMode, CONVERT_FORMAT, ConvertParams, ConvertPass, TransferPath};
use super::frames::{FetchPolicy, FrameId, FrameLookup, FrameProvider, FrameRef};
use super::gpu::{OPTIONAL_FEATURES, create_instance};
use super::offline::{FailedMedia, OfflineOptions, OfflineRenderer, OutputFormat};
use super::offscreen::{OUTPUT_FORMAT, Readback};
use super::plan::{Affine2, BackgroundPlan, LayerContent, LayerPlan, RenderPlan};
use super::present::PresentPass;
use crate::color::{ColorRange, YuvMatrix};
use crate::media::readers::{ReaderOptions, ReaderPool};
use crate::media::{CpuFrame, DecodedVideoFrame, FrameData, PlaneBuf, PlaneLayout, SampleDepth};

const TIMESTAMPS: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// The adapter `CUTFRAME_GPU` asks for, with timestamp queries when it has them.
fn timing_gpu() -> Arc<GpuContext> {
    let instance = create_instance();
    let integrated =
        std::env::var("CUTFRAME_GPU").is_ok_and(|v| v.eq_ignore_ascii_case("integrated"));
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: if integrated {
            wgpu::PowerPreference::LowPower
        } else {
            wgpu::PowerPreference::HighPerformance
        },
        ..Default::default()
    }))
    .expect("a GPU");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("timing"),
        required_features: adapter.features() & (OPTIONAL_FEATURES | TIMESTAMPS),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .expect("a device");
    let info = adapter.get_info();
    eprintln!(
        "adapter: {} ({:?}, driver {}), GPU timestamps: {}",
        info.name,
        info.device_type,
        info.driver_info,
        device.features().contains(TIMESTAMPS)
    );
    Arc::new(GpuContext {
        instance,
        adapter,
        device,
        queue,
        fallback: None,
    })
}

/// GPU time of the work recorded between [`Self::begin`] and [`Self::end`].
struct GpuTimer {
    queries: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer)>,
}

impl GpuTimer {
    fn new(gpu: &GpuContext) -> Self {
        let queries = gpu.device.features().contains(TIMESTAMPS).then(|| {
            let set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("timing"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
            let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timing.resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timing.read"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            (set, resolve, read)
        });
        Self { queries }
    }

    fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some((set, ..)) = &self.queries {
            encoder.write_timestamp(set, 0);
        }
    }

    fn end(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some((set, resolve, read)) = &self.queries {
            encoder.write_timestamp(set, 1);
            encoder.resolve_query_set(set, 0..2, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, read, 0, 16);
        }
    }

    /// Submits `encoder`, waits, and returns the GPU time between `begin` and `end`
    /// (timestamps), or the wall time of submit and wait without timestamps.
    fn submit(&self, gpu: &GpuContext, encoder: wgpu::CommandEncoder) -> Duration {
        let start = Instant::now();
        let index = gpu.queue.submit([encoder.finish()]);
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: None,
            })
            .expect("poll");
        let wall = start.elapsed();
        let Some((_, _, read)) = &self.queries else {
            return wall;
        };
        self.read(gpu, read)
    }

    fn read(&self, gpu: &GpuContext, read: &wgpu::Buffer) -> Duration {
        let slice = read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let ticks: [u64; 2] = {
            let view = slice.get_mapped_range().expect("mapped");
            bytemuck::pod_read_unaligned(&view[..16])
        };
        read.unmap();
        let ns =
            ticks[1].saturating_sub(ticks[0]) as f64 * f64::from(gpu.queue.get_timestamp_period());
        Duration::from_nanos(ns as u64)
    }

    /// The GPU time of a submission that already completed (`None` without timestamps).
    fn last(&self, gpu: &GpuContext) -> Option<Duration> {
        self.queries
            .as_ref()
            .map(|(_, _, read)| self.read(gpu, read))
    }
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn timing_clip() -> Option<PathBuf> {
    std::env::var_os("CUTFRAME_TIMING_CLIP").map(PathBuf::from)
}

fn clip_asset(path: &std::path::Path) -> MediaAsset {
    let probed = crate::media::probe(path).expect("probe");
    MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: crate::media::fingerprint(path).expect("fingerprint"),
        info: probed.info,
    }
}

/// Budget: convert of one 4K P010 frame to 1280 x 720 with the tone map, at most 2 ms GPU.
#[cfg(windows)]
#[test]
#[ignore = "timing: hardware GPU, release build and CUTFRAME_TIMING_CLIP"]
fn convert_of_a_4k_p010_frame_to_720p() {
    let Some(path) = timing_clip() else {
        eprintln!("CUTFRAME_TIMING_CLIP is not set");
        return;
    };
    let gpu = timing_gpu();
    let asset = clip_asset(&path);
    let mut readers = ReaderPool::new(Some(Arc::clone(&gpu)), ReaderOptions::default(), |_| {});
    let mut project = Project::new("timing");
    project.media = vec![asset.clone()];
    readers.set_project(&project);
    readers.begin_frame();
    let FrameLookup::Exact(frame) = readers.video(asset.id, Time::ZERO, FetchPolicy::Exact) else {
        panic!("no frame");
    };
    let FrameData::Gpu(surface) = frame.data() else {
        panic!("not a zero-copy frame: {:?}", readers.decode_path(asset.id));
    };
    assert_eq!(surface.texture.format(), wgpu::TextureFormat::P010);
    let params = ConvertParams::for_video(asset.info.video.as_ref().expect("video"));
    eprintln!(
        "frame: coded {:?}, visible {:?}, {:?}",
        surface.coded, surface.visible, params.transfer
    );
    let pass = ConvertPass::new(&gpu.device);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("timing.target"),
        size: wgpu::Extent3d {
            width: 1280,
            height: 720,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CONVERT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let timer = GpuTimer::new(&gpu);
    let mut times = Vec::new();
    for _ in 0..60 {
        let source = gpu_source(surface);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        timer.begin(&mut encoder);
        pass.encode(
            &gpu.device,
            &mut encoder,
            &source,
            &params,
            &view,
            (1280, 720),
        );
        timer.end(&mut encoder);
        surface.record_release(&mut encoder);
        times.push(timer.submit(&gpu, encoder));
    }
    eprintln!(
        "convert 4K P010 -> 1280x720 with tone map: median {}, min {} (budget 2 ms)",
        ms(median(times.clone())),
        ms(*times.iter().min().expect("times"))
    );
}

/// A provider of synthetic 8-bit 4:2:0 CPU frames, one source per media; with `fresh` every
/// composite gets new frames (new pts), so it uploads and converts again.
struct Synthetic {
    frames: Vec<(MediaId, CpuFrame)>,
    fresh: bool,
    pts: i64,
}

impl Synthetic {
    fn new(count: usize, size: (u32, u32), fresh: bool) -> Self {
        let (w, h) = size;
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let plane = |w: u32, h: u32, seed: u32| PlaneBuf {
            stride: w as usize,
            bytes: (0..w * h)
                .map(|i| ((i * 7 + seed * 31) % 220 + 16) as u8)
                .collect(),
        };
        let frames = (0..count)
            .map(|i| {
                let seed = i as u32;
                let frame = CpuFrame {
                    layout: PlaneLayout::Planar3 {
                        chroma_w: cw,
                        chroma_h: ch,
                    },
                    depth: SampleDepth::U8,
                    visible: size,
                    planes: [
                        Some(plane(w, h, seed)),
                        Some(plane(cw, ch, seed + 1)),
                        Some(plane(cw, ch, seed + 2)),
                    ],
                };
                (MediaId::new(), frame)
            })
            .collect();
        Self {
            frames,
            fresh,
            pts: 0,
        }
    }

    fn params(size: (u32, u32)) -> ConvertParams {
        ConvertParams {
            matrix: Some(YuvMatrix::Bt709),
            range: ColorRange::Limited,
            transfer: TransferPath::Sdr,
            rotation: Rotation::None,
            alpha: AlphaMode::Opaque,
            display_size: size,
        }
    }
}

impl FrameProvider for Synthetic {
    fn begin_frame(&mut self) {
        if self.fresh {
            self.pts += 1;
        }
    }

    fn video(&mut self, media: MediaId, _: Time, _: FetchPolicy) -> FrameLookup {
        let (_, frame) = self
            .frames
            .iter()
            .find(|(id, _)| *id == media)
            .expect("a synthetic source");
        let id = FrameId {
            media,
            source: 1,
            pts: Some(Time(self.pts)),
        };
        let decoded = DecodedVideoFrame {
            pts: Time(self.pts),
            data: FrameData::Cpu(frame.clone()),
        };
        FrameLookup::Exact(FrameRef::new(id, Arc::new(decoded)))
    }

    fn image(&mut self, _: MediaId, _: FetchPolicy) -> FrameLookup {
        FrameLookup::Missing
    }
}

/// A plan with one video layer per source of `provider`, each smaller than the one below and
/// all but the first half transparent.
fn plan_of(provider: &Synthetic, canvas: (u32, u32), render: (u32, u32), n: i64) -> RenderPlan {
    let (w, h) = (canvas.0 as f32, canvas.1 as f32);
    let layers = provider
        .frames
        .iter()
        .enumerate()
        .map(|(i, (media, frame))| {
            let inset = i as f32 * 0.1;
            LayerPlan {
                clip: ClipId::new(),
                content: LayerContent::Video {
                    media: *media,
                    sample_time: Time::ZERO,
                    convert: Synthetic::params(frame.visible),
                },
                transform: Affine2::rect(
                    w * inset,
                    h * inset,
                    w * (1.0 - 2.0 * inset),
                    h * (1.0 - 2.0 * inset),
                ),
                crop: Default::default(),
                opacity: if i == 0 { 1.0 } else { 0.5 },
            }
        })
        .collect();
    RenderPlan {
        frame: n,
        time: Time::ZERO,
        canvas,
        render_size: render,
        background: BackgroundPlan::Color(Rgba8::BLACK),
        layers,
    }
}

/// Budgets: composite + present of a 1080p canvas into a 1280 x 720 preview with 4 layers, at
/// most 4 ms GPU (sources already converted, the paused-edit case); an offline 1080p frame
/// excluding decode, at most 8 ms on the iGPU (upload, convert, composite, output pass and
/// read-back of a new source frame each time).
#[test]
#[ignore = "timing: hardware GPU and a release build"]
fn composite_present_and_offline_frame() {
    let gpu = timing_gpu();
    let timer = GpuTimer::new(&gpu);

    let canvas = (1920, 1080);
    let render = super::plan::render_size(canvas, (1280, 720), 1);
    let mut provider = Synthetic::new(4, canvas, false);
    let mut compositor = Compositor::new(&gpu);
    let (_canvas_texture, canvas_view) = create_canvas(&gpu.device, render);
    let surface = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("timing.surface"),
        size: wgpu::Extent3d {
            width: 1280,
            height: 720,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let mut present = PresentPass::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut times = Vec::new();
    for n in 0..120 {
        // A new frame index each time: the plan changes, the sources stay cached.
        let plan = plan_of(&provider, canvas, render, n);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        timer.begin(&mut encoder);
        let outcome = compositor.render(&gpu, &mut encoder, &plan, &mut provider, &canvas_view);
        assert!(outcome.drawn);
        present.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &canvas_view,
            render,
            &surface_view,
            (1280, 720),
        );
        timer.end(&mut encoder);
        let time = timer.submit(&gpu, encoder);
        compositor.submitted(&gpu);
        if n >= 10 {
            times.push(time);
        }
    }
    eprintln!(
        "composite {render:?} (1080p canvas, 4 layers, cached sources) + present into 1280x720: \
         median {}, min {} (budget 4 ms)",
        ms(median(times.clone())),
        ms(*times.iter().min().expect("times"))
    );

    let mut provider = Synthetic::new(1, canvas, true);
    let mut compositor = Compositor::with_config(&gpu, CompositorConfig::EXPORT);
    let (_texture, canvas_view) = create_canvas(&gpu.device, canvas);
    let mut output = PresentPass::new(&gpu.device, OUTPUT_FORMAT);
    let mut readback = Readback::new(&gpu.device, canvas);
    let mut wall = Vec::new();
    let mut gpu_times = Vec::new();
    for n in 0..60 {
        let start = Instant::now();
        let plan = plan_of(&provider, canvas, canvas, n);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        timer.begin(&mut encoder);
        compositor.render(&gpu, &mut encoder, &plan, &mut provider, &canvas_view);
        output.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &canvas_view,
            canvas,
            &readback.view,
            canvas,
        );
        timer.end(&mut encoder);
        readback.encode_copy(&mut encoder);
        let index = gpu.queue.submit([encoder.finish()]);
        let pixels = readback.read(&gpu, index).expect("read-back").len();
        assert_eq!(pixels, 1920 * 1080 * 4);
        let elapsed = start.elapsed();
        if n >= 5 {
            wall.push(elapsed);
            if let Some(time) = timer.last(&gpu) {
                gpu_times.push(time);
            }
        }
    }
    eprintln!(
        "offline 1080p frame (new 8-bit 4:2:0 source: upload, convert, composite, output, \
         read-back): wall median {}, GPU median {} (budget 8 ms on the iGPU)",
        ms(median(wall)),
        if gpu_times.is_empty() {
            "n/a".to_string()
        } else {
            ms(median(gpu_times))
        }
    );
}

/// 4K HEVC 10-bit decode + convert through `OfflineRenderer::render_frame` in sequence, at the
/// canvas size and at 1080p (frames per second, decode path).
#[test]
#[ignore = "timing: hardware GPU, release build and CUTFRAME_TIMING_CLIP"]
fn offline_4k_hevc_decode_and_convert_rate() {
    let Some(path) = timing_clip() else {
        eprintln!("CUTFRAME_TIMING_CLIP is not set");
        return;
    };
    let gpu = timing_gpu();
    let asset = clip_asset(&path);
    let display = asset.info.video.as_ref().expect("video").display_size();
    let duration = asset.info.duration.expect("duration");
    let mut project = Project::new("timing");
    project.settings.width = display.0;
    project.settings.height = display.1;
    project.settings.frame_rate = app_core::FrameRate::FPS_60;
    project.media = vec![asset.clone()];
    project.tracks[0].clips = vec![app_core::Clip {
        source: app_core::ClipSource::media_at(asset.id, Time::ZERO),
        ..app_core::Clip::color("clip", Time::ZERO, duration, Rgba8::BLACK)
    }];
    project.validate().expect("valid");
    let frames = duration.to_frames(project.settings.frame_rate).min(240);
    for (size, format) in [
        (display, OutputFormat::GpuCanvas),
        ((1920, 1080), OutputFormat::GpuCanvas),
        ((1920, 1080), OutputFormat::Rgba8),
    ] {
        let mut renderer = OfflineRenderer::new(
            Arc::clone(&gpu),
            OfflineOptions {
                failed_media: FailedMedia::Error,
                below_normal_priority: false,
                ..OfflineOptions::default()
            },
        );
        // Warm up: open the reader, decode the first frame.
        renderer
            .render_frame(&project, 0, size, format)
            .expect("frame 0");
        let start = Instant::now();
        for n in 1..frames {
            renderer
                .render_frame(&project, n, size, format)
                .expect("frame");
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
        }
        let elapsed = start.elapsed();
        eprintln!(
            "OfflineRenderer {size:?} {format:?}: {} frames in {:.2} s = {:.1} fps ({:?})",
            frames - 1,
            elapsed.as_secs_f64(),
            (frames - 1) as f64 / elapsed.as_secs_f64(),
            renderer.decode_path(asset.id)
        );
    }
}
