//! The D3D12VA zero-copy spike (render-graph.md §5.4, task 0320 step 1). Ignored by default:
//! it needs hardware video decode and the large clips are not in the repository.
//!
//! For each clip it measures, on the adapter `GpuContext::new` picks (`CUTFRAME_GPU`,
//! `WGPU_FORCE_FALLBACK_ADAPTER`):
//! - decode fps and process CPU time per frame of the zero-copy, download and software paths;
//! - the max per-channel difference (of 255) between a zero-copy frame converted on the GPU and
//!   the software-decoded frame of the same pts converted on the CPU with the same formula;
//! - a long loop that converts every frame on the GPU with frames held in flight like a reader
//!   queue, counting distinct decoder surfaces and frames per pass (pool exhaustion);
//! - ERROR and WARN events (the D3D12 debug layer's messages arrive through wgpu-hal's log).
//!
//! ```text
//! WGPU_VALIDATION=1 CUTFRAME_GPU=integrated CUTFRAME_SPIKE_FILES="a.mp4;b.mp4" \
//!   cargo test --release -p engine --lib d3d12::spike -- --ignored --nocapture
//! ```
//!
//! Without `CUTFRAME_SPIKE_FILES` the committed HEVC HLG and AV1 PQ fixtures are used.
//! `CUTFRAME_SPIKE_FRAMES` (default 300) sets the frames per measurement, `CUTFRAME_SPIKE_LOOP`
//! (default 3000) the long loop, `CUTFRAME_SPIKE_NO_RELEASE=1` skips
//! [`GpuFrame::record_release`] to show what the debug layer says without it.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use app_core::Time;
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

use super::*;
use crate::media::decode::open_decoder_with_threads;
use crate::media::reader::VideoReader;

static ERRORS: AtomicUsize = AtomicUsize::new(0);
static WARNINGS: AtomicUsize = AtomicUsize::new(0);

/// Counts ERROR and WARN events (wgpu-hal forwards D3D12 debug-layer messages to `log`).
struct CountLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CountLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        match *event.metadata().level() {
            tracing::Level::ERROR => ERRORS.fetch_add(1, Ordering::Relaxed),
            tracing::Level::WARN => WARNINGS.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
    }
}

fn init_logging() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .with(CountLayer)
            .try_init()
            .ok();
    });
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// User plus kernel time of the whole process (decoder, driver and wgpu threads included).
fn process_cpu_time() -> Duration {
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    // SAFETY: the current-process pseudo handle and four writable FILETIMEs.
    unsafe { GetProcessTimes(GetCurrentProcess(), creation, exit, kernel, user) }
        .expect("GetProcessTimes");
    let ticks = |t: &FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

struct Clip {
    path: PathBuf,
    stream: u32,
    origin: Time,
    fps: f64,
    kr: f32,
    kb: f32,
}

impl Clip {
    fn probe(path: &Path) -> Self {
        let info = crate::media::probe(path).expect("probe").info;
        let video = info.video.expect("a video stream");
        let matrix = video.color.matrix.clone().unwrap_or_default();
        let (kr, kb) = if matrix.contains("2020") {
            (0.2627, 0.0593)
        } else if matrix.contains("601") || matrix.contains("470") || video.height < 720 {
            (0.299, 0.114)
        } else {
            (0.2126, 0.0722)
        };
        Self {
            path: path.to_owned(),
            stream: video.stream_index,
            origin: info.origin,
            fps: video.frame_rate.map_or(30.0, |r| r.as_f64()),
            kr,
            kb,
        }
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Measure {
    frames: usize,
    wall: Duration,
    cpu: Duration,
}

impl Measure {
    fn fps(&self) -> f64 {
        self.frames as f64 / self.wall.as_secs_f64()
    }
    fn cpu_ms_per_frame(&self) -> f64 {
        self.cpu.as_secs_f64() * 1000.0 / self.frames as f64
    }
    /// Process CPU in % of one core when decoding at `fps` (the source rate).
    fn cpu_pct_at(&self, fps: f64) -> f64 {
        self.cpu_ms_per_frame() * fps / 10.0
    }
}

/// Decodes `n` frames (looping the clip) and hands each to `on_frame`. Returns the measurement
/// and the number of frames of every complete pass through the clip.
fn decode_loop(
    reader: &mut VideoReader<'_>,
    n: usize,
    mut on_frame: impl FnMut(Option<Time>, AVFrame) -> Result<(), String>,
) -> Result<(Measure, Vec<usize>), String> {
    let (start, cpu0) = (Instant::now(), process_cpu_time());
    let mut passes = Vec::new();
    let mut in_pass = 0;
    let mut frames = 0;
    while frames < n {
        match reader.next_frame().map_err(|e| e.to_string())? {
            Some((pts, frame)) => {
                on_frame(pts, frame)?;
                frames += 1;
                in_pass += 1;
            }
            None => {
                passes.push(in_pass);
                in_pass = 0;
                reader.rewind().map_err(|e| e.to_string())?;
            }
        }
    }
    let measure = Measure {
        frames,
        wall: start.elapsed(),
        cpu: process_cpu_time() - cpu0,
    };
    Ok((measure, passes))
}

// ---- YUV to RGB, GPU and CPU, same formula (BT.709 / BT.2020 matrix, limited range) ----------

const SHADER: &str = r"
struct Params { scale: f32, bits: f32, kr: f32, kb: f32, size: vec2<u32>, pad: vec2<u32> }
@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) { return; }
    let y = textureLoad(luma, vec2<i32>(id.xy), 0).r * p.scale;
    let c = textureLoad(chroma, vec2<i32>(id.xy / 2u), 0).rg * p.scale;
    let k = exp2(p.bits - 8.0);
    let yn = (y - 16.0 * k) / (219.0 * k);
    let cb = (c.x - 128.0 * k) / (224.0 * k);
    let cr = (c.y - 128.0 * k) / (224.0 * k);
    let r = yn + 2.0 * (1.0 - p.kr) * cr;
    let b = yn + 2.0 * (1.0 - p.kb) * cb;
    let g = (yn - p.kr * r - p.kb * b) / (1.0 - p.kr - p.kb);
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
";

fn yuv_to_rgb8(y: f32, cb: f32, cr: f32, bits: f32, kr: f32, kb: f32) -> [u8; 3] {
    let k = (bits - 8.0).exp2();
    let yn = (y - 16.0 * k) / (219.0 * k);
    let cb = (cb - 128.0 * k) / (224.0 * k);
    let cr = (cr - 128.0 * k) / (224.0 * k);
    let r = yn + 2.0 * (1.0 - kr) * cr;
    let b = yn + 2.0 * (1.0 - kb) * cb;
    let g = (yn - kr * r - kb * b) / (1.0 - kr - kb);
    [r, g, b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// RGB of a CPU frame (yuv420p, yuv420p10le, nv12 or p010le) converted on the CPU.
fn cpu_convert(frame: &AVFrame, clip: &Clip) -> (u32, u32, Vec<u8>) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let format = frame.format;
    let read8 = |plane: usize, x: usize, y: usize| {
        // SAFETY: a plane of a decoded frame, `linesize` bytes per row; callers stay inside the
        // frame (chroma at half resolution).
        unsafe { f32::from(*frame.data[plane].add(y * frame.linesize[plane] as usize + x)) }
    };
    let read16 = |plane: usize, x: usize, y: usize| {
        // SAFETY: as for `read8`, two bytes per sample.
        unsafe {
            let p = frame.data[plane].add(y * frame.linesize[plane] as usize + 2 * x);
            u16::from_le_bytes([*p, *p.add(1)])
        }
    };
    type Sampler<'a> = Box<dyn Fn(usize, usize) -> (f32, f32, f32) + 'a>;
    let (bits, sample): (f32, Sampler<'_>) = match format {
        ffi::AV_PIX_FMT_YUV420P => (
            8.0,
            Box::new(|x, y| {
                (
                    read8(0, x, y),
                    read8(1, x / 2, y / 2),
                    read8(2, x / 2, y / 2),
                )
            }),
        ),
        ffi::AV_PIX_FMT_YUV420P10LE => (
            10.0,
            Box::new(|x, y| {
                (
                    f32::from(read16(0, x, y)),
                    f32::from(read16(1, x / 2, y / 2)),
                    f32::from(read16(2, x / 2, y / 2)),
                )
            }),
        ),
        ffi::AV_PIX_FMT_NV12 => (
            8.0,
            Box::new(|x, y| {
                (
                    read8(0, x, y),
                    read8(1, x / 2 * 2, y / 2),
                    read8(1, x / 2 * 2 + 1, y / 2),
                )
            }),
        ),
        ffi::AV_PIX_FMT_P010LE => (
            10.0,
            Box::new(|x, y| {
                (
                    f32::from(read16(0, x, y) >> 6),
                    f32::from(read16(1, x / 2 * 2, y / 2) >> 6),
                    f32::from(read16(1, x / 2 * 2 + 1, y / 2) >> 6),
                )
            }),
        ),
        other => panic!("spike: unsupported CPU pixel format {other}"),
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (luma, cb, cr) = sample(x, y);
            let [r, g, b] = yuv_to_rgb8(luma, cb, cr, bits, clip.kr, clip.kb);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    (w as u32, h as u32, rgba)
}

/// The GPU convert of the spike: one compute pass from the two plane views to `Rgba8Unorm`.
struct Converter {
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    target: Option<(u32, u32, wgpu::Texture)>,
    release: bool,
}

impl Converter {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike convert"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("spike convert"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spike params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            params,
            target: None,
            release: std::env::var("CUTFRAME_SPIKE_NO_RELEASE").is_err(),
        }
    }

    /// Converts `frame` (visible area) and submits; with `readback`, returns the RGBA8 pixels.
    fn convert(
        &mut self,
        gpu: &GpuContext,
        frame: &GpuFrame,
        clip: &Clip,
        readback: bool,
    ) -> (wgpu::SubmissionIndex, Option<Vec<u8>>) {
        let (w, h) = frame.visible;
        if self.target.as_ref().is_none_or(|t| (t.0, t.1) != (w, h)) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spike rgba"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.target = Some((w, h, texture));
        }
        let target = &self.target.as_ref().expect("target").2;
        let ten_bit = frame.texture.format() == wgpu::TextureFormat::P010;
        let (scale, bits) = if ten_bit {
            (65535.0 / 64.0, 10.0)
        } else {
            (255.0, 8.0)
        };
        let mut params = Vec::with_capacity(32);
        for v in [scale, bits, clip.kr, clip.kb] {
            params.extend_from_slice(&f32::to_le_bytes(v));
        }
        for v in [w, h, 0, 0] {
            params.extend_from_slice(&u32::to_le_bytes(v));
        }
        gpu.queue.write_buffer(&self.params, 0, &params);
        let [luma, chroma] = frame.plane_views();
        let out = target.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spike convert"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&luma),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&chroma),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        if self.release {
            frame.record_release(&mut encoder);
        }
        let padded_row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = readback.then(|| {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spike readback"),
                size: u64::from(padded_row * h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            buffer
        });
        let index = gpu.queue.submit([encoder.finish()]);
        let pixels = buffer.map(|buffer| {
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index.clone()),
                    timeout: None,
                })
                .expect("poll");
            let mapped = slice.get_mapped_range().expect("mapped");
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for row in mapped.chunks_exact(padded_row as usize) {
                rgba.extend_from_slice(&row[..(w * 4) as usize]);
            }
            rgba
        });
        (index, pixels)
    }
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len(), "frame sizes differ");
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

// ---- The measurements -------------------------------------------------------------------------

fn open_hw<'c>(
    clip: &Clip,
    device: &HwDevice,
    cancel: &'c AtomicBool,
) -> Result<VideoReader<'c>, String> {
    VideoReader::open_with(&clip.path, clip.stream, clip.origin, cancel, |s| {
        open_hw_decoder(s, device)
    })
    .map_err(|e| e.to_string())
}

fn open_sw<'c>(clip: &Clip, cancel: &'c AtomicBool) -> VideoReader<'c> {
    // The software fallback's thread count (render-graph.md §5.4): clamp(cores / 2, 2, 8).
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get()) as i32;
    VideoReader::open_with(&clip.path, clip.stream, clip.origin, cancel, |s| {
        open_decoder_with_threads(s, (cores / 2).clamp(2, 8))
    })
    .expect("software decoder")
}

/// Reference: the software frame number `index` and its pts, converted on the CPU.
fn software_reference(clip: &Clip, index: usize) -> (Option<Time>, (u32, u32, Vec<u8>)) {
    let cancel = AtomicBool::new(false);
    let mut reader = open_sw(clip, &cancel);
    for _ in 0..index {
        reader.next_frame().expect("decode").expect("enough frames");
    }
    let (pts, frame) = reader.next_frame().expect("decode").expect("enough frames");
    (pts, cpu_convert(&frame, clip))
}

/// The hardware frame with `pts`, decoded from the start.
fn hw_frame_at(reader: &mut VideoReader<'_>, pts: Option<Time>) -> AVFrame {
    loop {
        let (time, frame) = reader
            .next_frame()
            .expect("decode")
            .expect("frame with pts");
        if time == pts {
            return frame;
        }
    }
}

fn report_clip(gpu: &GpuContext, device: Option<&HwDevice>, clip: &Clip) {
    let n = env_usize("CUTFRAME_SPIKE_FRAMES", 300);
    let long = env_usize("CUTFRAME_SPIKE_LOOP", 3000);
    let compare_index = env_usize("CUTFRAME_SPIKE_COMPARE_INDEX", 5);
    let (errors0, warnings0) = (
        ERRORS.load(Ordering::Relaxed),
        WARNINGS.load(Ordering::Relaxed),
    );
    println!("\n=== {} ({} fps source)", clip.name(), clip.fps);
    let cancel = AtomicBool::new(false);

    // Software.
    let mut sw = open_sw(clip, &cancel);
    let (m, _) = decode_loop(&mut sw, n, |_, _| Ok(())).expect("software decode");
    println!(
        "software : {:7.1} fps, {:6.2} ms CPU/frame, {:6.1} % of one core at source rate",
        m.fps(),
        m.cpu_ms_per_frame(),
        m.cpu_pct_at(clip.fps)
    );
    report_debug_layer(gpu);
    drop(sw);

    let Some(device) = device else {
        println!("hardware : unavailable, path = software");
        return;
    };
    let mut reader = match open_hw(clip, device, &cancel) {
        Ok(reader) => reader,
        Err(err) => {
            println!("hardware : decoder does not open ({err}), path = software");
            return;
        }
    };
    let wrapper = FrameWrapper::new(gpu).expect("frame wrapper");

    // Download.
    let mut hw_frames = 0usize;
    let result = decode_loop(&mut reader, n, |_, frame| {
        if is_hw_frame(&frame) {
            hw_frames += 1;
            download(&frame).map(drop).map_err(|e| e.to_string())
        } else {
            Ok(())
        }
    });
    let m = match result {
        Ok((m, _)) => m,
        Err(err) if hw_frames == 0 => {
            println!(
                "hardware : decode fails ({err}), hw_active = {}, device removed: {:?}, path = software",
                hw_active(reader.decoder()),
                device_removed_reason(gpu)
            );
            report_debug_layer(gpu);
            return;
        }
        Err(err) => hw_failed(gpu, &err),
    };
    if hw_frames == 0 {
        println!("hardware : decoder opened but produced software frames, path = software");
        return;
    }
    println!(
        "download : {:7.1} fps, {:6.2} ms CPU/frame, {:6.1} % of one core at source rate",
        m.fps(),
        m.cpu_ms_per_frame(),
        m.cpu_pct_at(clip.fps)
    );
    report_debug_layer(gpu);

    // Zero-copy decode + fence wait + wrap.
    reader.rewind().expect("rewind");
    let mut wrap_error = None;
    let (m, _) = decode_loop(&mut reader, n, |_, frame| {
        match wrapper.wrap(frame) {
            Ok(frame) => drop(frame),
            Err((_, err)) => wrap_error = Some(err.to_string()),
        }
        Ok(())
    })
    .unwrap_or_else(|err| hw_failed(gpu, &err));
    if let Some(err) = wrap_error {
        println!("zero-copy: wrap failed ({err}), path = download");
        return;
    }
    println!(
        "zero-copy: {:7.1} fps, {:6.2} ms CPU/frame, {:6.1} % of one core at source rate (decode + fence + wrap)",
        m.fps(),
        m.cpu_ms_per_frame(),
        m.cpu_pct_at(clip.fps)
    );
    report_debug_layer(gpu);

    // Pixel comparison with the software decode of the same pts.
    let (pts, (sw_w, sw_h, sw_rgb)) = software_reference(clip, compare_index);
    let mut converter = Converter::new(&gpu.device);
    let mut cmp = open_hw(clip, device, &cancel).expect("hardware decoder");
    let frame = hw_frame_at(&mut cmp, pts);
    let (_, _, dl_rgb) = cpu_convert(&download(&frame).expect("download"), clip);
    let frame = wrapper.wrap(frame).map_err(|(_, e)| e).expect("wrap");
    let format = frame.texture.format();
    let (coded, visible) = (frame.coded, frame.visible);
    let (_, gpu_rgb) = converter.convert(gpu, &frame, clip, true);
    drop(frame);
    drop(cmp);
    assert_eq!(visible, (sw_w, sw_h), "visible size");
    println!(
        "compare  : frame {compare_index} ({format:?}, coded {coded:?}, visible {visible:?}): \
         zero-copy GPU vs software CPU max diff {}/255, download CPU vs software CPU {}/255",
        max_diff(&gpu_rgb.expect("pixels"), &sw_rgb),
        max_diff(&dl_rgb, &sw_rgb)
    );
    report_debug_layer(gpu);

    // Long loop: convert every frame, keep READER_QUEUE_DEPTH frames in flight.
    reader.rewind().expect("rewind");
    let mut surfaces = HashSet::new();
    let mut in_flight: VecDeque<(wgpu::SubmissionIndex, GpuFrame)> = VecDeque::new();
    let (m, passes) = decode_loop(&mut reader, long, |_, frame| {
        if let Some(id) = surface_id(&frame) {
            surfaces.insert(id);
        }
        let frame = wrapper.wrap(frame).map_err(|(_, e)| e.to_string())?;
        let (index, _) = converter.convert(gpu, &frame, clip, false);
        in_flight.push_back((index, frame));
        while in_flight.len() > READER_QUEUE_DEPTH {
            let (index, frame) = in_flight.pop_front().expect("frame");
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .map_err(|e| e.to_string())?;
            drop(frame);
        }
        Ok(())
    })
    .unwrap_or_else(|err| hw_failed(gpu, &err));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    in_flight.clear();
    println!(
        "long loop: {} frames, {:7.1} fps with GPU convert, {:6.2} ms CPU/frame, \
         {} distinct surfaces, frames per pass {:?}",
        m.frames,
        m.fps(),
        m.cpu_ms_per_frame(),
        surfaces.len(),
        passes
    );
    report_debug_layer(gpu);
    println!(
        "log      : {} errors, {} warnings during this clip",
        ERRORS.load(Ordering::Relaxed) - errors0,
        WARNINGS.load(Ordering::Relaxed) - warnings0
    );
}

#[test]
#[ignore = "hardware video spike: needs a GPU with D3D12 video decode, run with --ignored"]
fn spike_report() {
    init_logging();
    // `CUTFRAME_SPIKE_AVLOG=48` shows FFmpeg's debug log (hwaccel setup).
    if let Some(level) = std::env::var("CUTFRAME_SPIKE_AVLOG")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        // SAFETY: sets a global log level.
        unsafe { ffi::av_log_set_level(level) };
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media");
    let files: Vec<PathBuf> = match std::env::var("CUTFRAME_SPIKE_FILES") {
        Ok(list) => list
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => vec![
            fixtures.join("hevc-hlg-dv84.mov"),
            fixtures.join("av1-10bit-pq.mp4"),
        ],
    };
    let gpu = GpuContext::new().expect("GPU");
    let info = gpu.info();
    println!(
        "adapter {} ({:?}, {}), zero-copy formats: {}, validation env: {:?}",
        info.adapter,
        info.device_type,
        info.driver,
        gpu.supports_zero_copy_video(),
        std::env::var("WGPU_VALIDATION").ok()
    );
    println!(
        "debug layer: {}",
        if take_debug_messages(&gpu).is_some() {
            "on (ID3D12InfoQueue present)"
        } else {
            "off"
        }
    );
    let device = match HwDevice::new(&gpu) {
        Ok(device) => Some(device),
        Err(err) => {
            println!("no D3D12VA device: {err}");
            None
        }
    };
    for file in &files {
        report_clip(&gpu, device.as_ref(), &Clip::probe(file));
    }
    println!(
        "\ntotal log: {} errors, {} warnings",
        ERRORS.load(Ordering::Relaxed),
        WARNINGS.load(Ordering::Relaxed)
    );
}

/// Reports a hardware decode failure (with the device-removed reason, if any) and ends the test.
fn hw_failed(gpu: &GpuContext, err: &str) -> ! {
    panic!(
        "hardware decode failed: {err}; device removed: {:?}",
        device_removed_reason(gpu)
    );
}

/// Prints the D3D12 debug-layer messages stored since the last call (errors and warnings in
/// full, up to three errors and two warnings).
fn report_debug_layer(gpu: &GpuContext) {
    let Some(messages) = take_debug_messages(gpu) else {
        println!("d3d12    : debug layer off");
        return;
    };
    let errors: Vec<_> = messages.iter().filter(|m| m.severity <= 1).collect();
    let warnings: Vec<_> = messages.iter().filter(|m| m.severity == 2).collect();
    println!(
        "d3d12    : {} errors, {} warnings, {} messages in total",
        errors.len(),
        warnings.len(),
        messages.len()
    );
    for m in errors.iter().take(3).chain(warnings.iter().take(2)) {
        println!("  [{}] {}", m.severity, m.text);
    }
}
