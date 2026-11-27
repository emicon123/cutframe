//! The export loop end to end (`engine::export::run`, export.md §11 "GPU tests on WARP"): real
//! exports of small test projects on the WARP adapter with OpenH264, then the file is demuxed
//! and decoded with FFmpeg.
//!
//! - **Golden export:** every decoded frame against the NV12 planes of the offline render of the
//!   same frame (PSNR-Y >= 40 dB, PSNR-Cb/Cr >= 38 dB at High quality), plus the container facts
//!   (faststart, H.264 High, AAC-LC stereo, frame count, CFR).
//! - **A/V sync**, **cancel**, **offline media**, **encoder fallback** before the first packet,
//!   **encoder failure** after it, **disk full** (`ENOSPC` from a fault-injecting writer, and the
//!   free-space floor), a refused overwrite.
//! - Ignored: a 1-minute 1080p30 export and the budget runs of export.md §10 on real hardware
//!   (`CUTFRAME_EXPORT_BENCH_SOURCE`, `CUTFRAME_ENCODERS`, `CUTFRAME_GPU`).
//!
//! Output files go to a per-test temp folder that is deleted afterwards.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use app_core::media::MediaPath;
use app_core::model::ClipId;
use app_core::{
    Animated, AudioProps, CanvasBackground, Clip, ClipSource, FLICKS_PER_SECOND, FrameRate,
    MediaAsset, MediaId, Project, Rgba8, Time, Track, TrackKind,
};
use engine::export::encoders::{
    EncoderCandidate, EncoderError, EncoderKind, EncoderSettings, OptionSet, open_encoder,
};
use engine::export::nv12::Nv12Pass;
use engine::export::output::partial_path;
use engine::export::plan::{
    ExportQuality, ExportResolution, VideoCodec, VideoExportSettings, plan_video_export,
};
use engine::export::run::{
    ExportError, ExportErrorKind, ExportHooks, ExportObserver, ExportRequest, ExportStage,
    ExportSummary, VideoEncoder, run_export, run_export_with,
};
use engine::media::{fingerprint, probe};
use engine::render::{
    AdapterMode, GpuContext, OfflineOptions, OfflineRenderer, OutputFormat, RenderedFrame,
};
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

/// WARP, shared by the tests of this binary.
fn warp() -> Arc<GpuContext> {
    static GPU: OnceLock<Arc<GpuContext>> = OnceLock::new();
    Arc::clone(GPU.get_or_init(|| {
        Arc::new(
            GpuContext::new_with(AdapterMode::SoftwareOnly)
                .expect("WARP (the D3D12 software adapter) ships with Windows"),
        )
    }))
}

/// Exports run one at a time: they share WARP and its CPU cores.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media")
        .join(name)
}

fn asset_at(path: &Path) -> MediaAsset {
    let probed = probe(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(path).expect("fingerprint"),
        info: probed.info,
    }
}

fn ms(milliseconds: i64) -> Time {
    Time(milliseconds * FLICKS_PER_SECOND / 1000)
}

fn media_clip(asset: &MediaAsset, start: Time, duration: Time) -> Clip {
    Clip {
        source: ClipSource::media_at(asset.id, Time::ZERO),
        ..Clip::color("Media", start, duration, Rgba8::BLACK)
    }
}

fn audio_clip(asset: &MediaAsset, start: Time, duration: Time, volume: f32) -> Clip {
    Clip {
        id: ClipId::new(),
        name: "Audio".into(),
        start,
        duration,
        source: ClipSource::media_at(asset.id, Time::ZERO),
        link: None,
        visual: None,
        audio: Some(AudioProps {
            volume: Animated::constant(volume),
            ..AudioProps::default()
        }),
    }
}

/// A folder in the temp directory, deleted on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cutframe-export-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Every file name in the folder, sorted.
    fn listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Some(keep) = std::env::var_os("CUTFRAME_EXPORT_TEST_KEEP") {
            for name in self.listing() {
                let _ = std::fs::copy(self.0.join(&name), Path::new(&keep).join(&name));
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Records what the loop reports; cancels once `cancel_after` frames were done.
#[derive(Default)]
struct Observer {
    stages: Mutex<Vec<ExportStage>>,
    total: AtomicU32,
    done: AtomicU32,
    cancel_after: Option<u32>,
    cancelled_at: Mutex<Option<Instant>>,
    playing: AtomicBool,
}

impl Observer {
    fn cancelling_after(frames: u32) -> Self {
        Self {
            cancel_after: Some(frames),
            ..Self::default()
        }
    }

    fn stages(&self) -> Vec<ExportStage> {
        let mut stages = self.stages.lock().unwrap().clone();
        stages.dedup();
        stages
    }
}

impl ExportObserver for Observer {
    fn cancelled(&self) -> bool {
        let hit = self
            .cancel_after
            .is_some_and(|n| self.done.load(Ordering::SeqCst) >= n);
        if hit {
            self.cancelled_at
                .lock()
                .unwrap()
                .get_or_insert_with(Instant::now);
        }
        hit
    }

    fn stage(&self, stage: ExportStage) {
        self.stages.lock().unwrap().push(stage);
    }

    fn set_total(&self, frames: u32) {
        self.total.store(frames, Ordering::SeqCst);
    }

    fn frame_done(&self) {
        self.done.fetch_add(1, Ordering::SeqCst);
    }

    fn playback_active(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }
}

fn openh264() -> EncoderCandidate {
    EncoderCandidate {
        kind: EncoderKind::OpenH264,
        codec: VideoCodec::H264,
        options: OptionSet::Full,
    }
}

fn settings(quality: ExportQuality) -> VideoExportSettings {
    VideoExportSettings {
        codec: VideoCodec::H264,
        resolution: ExportResolution::Project,
        frame_rate: None,
        quality,
        encoder: None,
    }
}

fn request(project: Project, quality: ExportQuality, output: PathBuf) -> ExportRequest {
    let plan = plan_video_export(&project, &settings(quality)).expect("a plan");
    ExportRequest {
        project: Arc::new(project),
        plan,
        candidates: vec![openh264()],
        explicit_encoder: false,
        output,
        overwrite: false,
    }
}

fn failed(result: Result<ExportSummary, ExportError>) -> engine::export::run::ExportFailure {
    match result {
        Err(ExportError::Failed(failure)) => failure,
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// The golden scene of the end-to-end test: 640 x 360, 30 fps, 2 s. H.264 video (0-1 s), an
/// HLG HEVC clip (1-1.3 s, tone-mapped) and a frame-numbered MPEG-4 clip after a cut, a PNG
/// with alpha over them (0.5-1.5 s), and a 1 kHz sine (0-1 s).
fn golden_project() -> Project {
    let h264 = asset_at(&fixture("h264-aac.mp4"));
    let hlg = asset_at(&fixture("hevc-hlg-dv84.mov"));
    let numbered = asset_at(&fixture("numbered-gop.mp4"));
    let image = asset_at(&fixture("image-alpha.png"));
    let sine = asset_at(&fixture("sine-1k-48k.flac"));
    let mut project = Project::new("Export golden");
    project.settings.width = 640;
    project.settings.height = 360;
    project.settings.frame_rate = FrameRate::FPS_30;
    project.background = CanvasBackground::Color {
        color: Rgba8::rgb(0x20, 0x40, 0x60),
    };
    project.tracks[0].clips = vec![
        media_clip(&h264, Time::ZERO, ms(1000)),
        media_clip(&hlg, ms(1000), ms(300)),
        media_clip(&numbered, ms(1300), ms(700)),
    ];
    project.tracks[1].clips = vec![audio_clip(&sine, Time::ZERO, ms(1000), 0.5)];
    let mut overlay = Track::new(TrackKind::Video, "Overlay");
    overlay.clips = vec![media_clip(&image, ms(500), ms(1000))];
    project.tracks.insert(0, overlay);
    project.media = vec![h264, hlg, numbered, image, sine];
    project.validate().expect("a valid project");
    project
}

/// Planes of one frame: Y' (w x h) and Cb, Cr (w/2 x h/2).
struct Planes {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
}

/// The NV12 planes the export pass makes of each offline render of `project` (the reference
/// the encoded frames are compared with).
fn reference_planes(gpu: &Arc<GpuContext>, project: &Project, frames: u32) -> Vec<Planes> {
    let size = (project.settings.width, project.settings.height);
    let mut renderer = OfflineRenderer::new(Arc::clone(gpu), OfflineOptions::default());
    let mut pass = Nv12Pass::new(Arc::clone(gpu), size);
    let mut out = Vec::new();
    for n in 0..frames {
        let canvas =
            match renderer.render_frame(project, i64::from(n), size, OutputFormat::GpuCanvas) {
                Ok(RenderedFrame::Gpu(view)) => view,
                other => panic!("frame {n}: {other:?}"),
            };
        pass.submit(canvas, u64::from(n)).unwrap();
        let frame = pass.next_ready(true).unwrap().expect("a mapped frame");
        let (w, h) = frame.size();
        let mut planes = Planes {
            y: Vec::new(),
            cb: Vec::new(),
            cr: Vec::new(),
        };
        for y in 0..h {
            planes.y.extend_from_slice(frame.luma_row(y));
        }
        for j in 0..h.div_ceil(2) {
            for pair in frame.chroma_row(j).as_chunks::<2>().0 {
                planes.cb.push(pair[0]);
                planes.cr.push(pair[1]);
            }
        }
        assert_eq!(planes.y.len(), (w * h) as usize);
        out.push(planes);
    }
    out
}

/// Decodes stream `index` of `path` (software decoders).
fn decode(path: &Path, index: usize) -> (Vec<AVFrame>, ffi::AVRational) {
    let name = CString::new(path.to_str().unwrap()).unwrap();
    let mut input = AVFormatContextInput::open(&name).expect("open the export");
    let (mut decoder, time_base) = {
        let stream = &input.streams()[index];
        let par = stream.codecpar();
        let codec = AVCodec::find_decoder(par.codec_id).expect("a decoder");
        let mut decoder = AVCodecContext::new(&codec);
        decoder.apply_codecpar(&par).unwrap();
        decoder.set_pkt_timebase(stream.time_base);
        (decoder, stream.time_base)
    };
    decoder.open(None).unwrap();
    let mut frames = Vec::new();
    let mut receive = |decoder: &mut AVCodecContext| loop {
        match decoder.receive_frame() {
            Ok(frame) => frames.push(frame),
            Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => break,
            Err(err) => panic!("decode: {err}"),
        }
    };
    while let Some(packet) = input.read_packet().unwrap() {
        if packet.stream_index as usize == index {
            decoder.send_packet(Some(&packet)).unwrap();
            receive(&mut decoder);
        }
    }
    decoder.send_packet(None).unwrap();
    receive(&mut decoder);
    (frames, time_base)
}

/// The planes of a decoded `yuv420p` frame.
fn decoded_planes(frame: &AVFrame) -> Planes {
    assert_eq!(frame.format, ffi::AV_PIX_FMT_YUV420P);
    let (w, h) = (frame.width as usize, frame.height as usize);
    let plane = |p: usize, width: usize, rows: usize| {
        let stride = frame.linesize[p] as usize;
        let mut out = Vec::with_capacity(width * rows);
        for row in 0..rows {
            // SAFETY: a decoded frame holds `rows` rows of `stride >= width` bytes in plane `p`.
            let line =
                unsafe { std::slice::from_raw_parts(frame.data[p].add(row * stride), width) };
            out.extend_from_slice(line);
        }
        out
    };
    Planes {
        y: plane(0, w, h),
        cb: plane(1, w.div_ceil(2), h.div_ceil(2)),
        cr: plane(2, w.div_ceil(2), h.div_ceil(2)),
    }
}

/// PSNR in dB of `actual` against `reference` over all their samples.
fn psnr(pairs: impl Iterator<Item = (u8, u8)>) -> f64 {
    let (mut sum, mut count) = (0f64, 0u64);
    for (a, b) in pairs {
        let d = f64::from(a) - f64::from(b);
        sum += d * d;
        count += 1;
    }
    let mse = sum / count.max(1) as f64;
    if mse == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

/// Top-level MP4 box types in file order.
fn top_level_boxes(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let mut boxes = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as u64;
        let kind = String::from_utf8_lossy(&bytes[pos + 4..pos + 8]).into_owned();
        let size = match size {
            0 => (bytes.len() - pos) as u64,
            1 => u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().unwrap()),
            size => size,
        };
        boxes.push(kind);
        pos += size as usize;
    }
    boxes
}

#[test]
fn the_export_matches_the_offline_render_and_is_a_youtube_ready_mp4() {
    let _serial = serial();
    let gpu = warp();
    let dir = TempDir::new("golden");
    let target = dir.file("Golden export.mp4");
    let project = golden_project();
    let req = request(project.clone(), ExportQuality::High, target.clone());
    let plan = req.plan;
    assert_eq!((plan.width, plan.height, plan.frames), (640, 360, 60));
    let obs = Observer::default();
    let summary = run_export(Arc::clone(&gpu), req, &obs).expect("the export");
    println!("{summary:?}, plan {plan:?}");

    // The job's view: stages in order, every frame counted once, only the target is left.
    assert_eq!(
        obs.stages(),
        [
            ExportStage::Preparing,
            ExportStage::Encoding,
            ExportStage::Finalizing
        ]
    );
    assert_eq!(obs.total.load(Ordering::SeqCst), 60);
    assert_eq!(obs.done.load(Ordering::SeqCst), 60);
    assert_eq!(dir.listing(), ["Golden export.mp4"]);
    assert_eq!(summary.path, target);
    assert_eq!(summary.encoder, EncoderKind::OpenH264);
    assert_eq!(summary.bytes, std::fs::metadata(&target).unwrap().len());

    // Container: faststart, H.264 High + AAC-LC stereo 48 kHz, CFR, 60 frames.
    let boxes = top_level_boxes(&target);
    assert!(
        boxes.iter().position(|b| b == "moov") < boxes.iter().position(|b| b == "mdat"),
        "faststart: {boxes:?}"
    );
    {
        let name = CString::new(target.to_str().unwrap()).unwrap();
        let input = AVFormatContextInput::open(&name).unwrap();
        let streams = input.streams();
        assert_eq!(streams.len(), 2);
        let video = streams[0].codecpar();
        assert_eq!(video.codec_id, ffi::AV_CODEC_ID_H264);
        assert_eq!(video.profile, ffi::AV_PROFILE_H264_HIGH as i32);
        assert_eq!((video.width, video.height), (640, 360));
        assert_eq!(
            [
                video.color_primaries as u32,
                video.color_trc as u32,
                video.color_space as u32,
                video.color_range as u32
            ],
            [
                ffi::AVCOL_PRI_BT709 as u32,
                ffi::AVCOL_TRC_BT709 as u32,
                ffi::AVCOL_SPC_BT709 as u32,
                ffi::AVCOL_RANGE_MPEG as u32
            ]
        );
        let audio = streams[1].codecpar();
        assert_eq!(audio.codec_id, ffi::AV_CODEC_ID_AAC);
        assert_eq!(audio.profile, ffi::AV_PROFILE_AAC_LOW as i32);
        assert_eq!(
            (audio.sample_rate, audio.ch_layout.nb_channels),
            (48_000, 2)
        );
    }

    // Pixels: every decoded frame against the NV12 planes of the offline render.
    let (decoded, time_base) = decode(&target, 0);
    assert_eq!(decoded.len(), 60, "one frame per plan frame");
    let mut pts: Vec<i64> = decoded.iter().map(|f| f.pts).collect();
    pts.sort_unstable();
    let step = pts[1] - pts[0];
    assert!(pts.windows(2).all(|w| w[1] - w[0] == step), "CFR: {pts:?}");
    assert_eq!(
        i64::from(time_base.den) / (step * i64::from(time_base.num)),
        30,
        "30 fps"
    );
    let reference = reference_planes(&gpu, &project, plan.frames);
    let actual: Vec<Planes> = decoded.iter().map(decoded_planes).collect();
    let pairs = |pick: fn(&Planes) -> &[u8]| {
        let reference = &reference;
        actual
            .iter()
            .zip(reference)
            .flat_map(move |(a, r)| pick(a).iter().copied().zip(pick(r).iter().copied()))
    };
    let (y, cb, cr) = (
        psnr(pairs(|p| &p.y)),
        psnr(pairs(|p| &p.cb)),
        psnr(pairs(|p| &p.cr)),
    );
    let worst_y = actual
        .iter()
        .zip(&reference)
        .map(|(a, r)| psnr(a.y.iter().copied().zip(r.y.iter().copied())))
        .fold(f64::INFINITY, f64::min);
    println!("PSNR Y {y:.2} dB (worst frame {worst_y:.2}), Cb {cb:.2} dB, Cr {cr:.2} dB");
    assert!(y >= 40.0, "PSNR-Y {y:.2} dB");
    assert!(cb >= 38.0 && cr >= 38.0, "PSNR-Cb {cb:.2}, Cr {cr:.2} dB");
}

#[test]
fn audio_and_video_stay_in_sync() {
    let _serial = serial();
    let gpu = warp();
    let dir = TempDir::new("sync");
    let sine = asset_at(&fixture("sine-1k-48k.flac"));
    let mut project = Project::new("Sync");
    project.settings.width = 160;
    project.settings.height = 90;
    project.settings.frame_rate = FrameRate::FPS_30;
    project.tracks[0].clips = vec![
        Clip::color("Black", Time::ZERO, ms(1000), Rgba8::BLACK),
        Clip::color("White", ms(1000), ms(1000), Rgba8::rgb(0xff, 0xff, 0xff)),
    ];
    project.tracks[1].clips = vec![audio_clip(&sine, ms(1000), ms(1000), 0.5)];
    project.media = vec![sine];
    project.validate().unwrap();
    let target = dir.file("sync.mp4");
    run_export(
        Arc::clone(&gpu),
        request(project, ExportQuality::Recommended, target.clone()),
        &Observer::default(),
    )
    .expect("the export");

    let (video, vtb) = decode(&target, 0);
    let white = video
        .iter()
        .filter(|f| {
            // SAFETY: plane 0 of a decoded frame has at least one byte.
            unsafe { *f.data[0] > 128 }
        })
        .map(|f| f.pts)
        .min()
        .expect("a white frame");
    let white_s = white as f64 * f64::from(vtb.num) / f64::from(vtb.den);
    let (audio, atb) = decode(&target, 1);
    let mut tone_s = None;
    for frame in &audio {
        // SAFETY: a decoded `fltp` frame holds `nb_samples` `f32`s in `data[0]`.
        let left = unsafe {
            std::slice::from_raw_parts(frame.data[0].cast::<f32>(), frame.nb_samples as usize)
        };
        if let Some(i) = left.iter().position(|s| s.abs() > 0.05) {
            let start = frame.pts as f64 * f64::from(atb.num) / f64::from(atb.den);
            tone_s = Some(start + i as f64 / 48_000.0);
            break;
        }
    }
    let tone_s = tone_s.expect("an audible sample");
    println!("first white frame at {white_s:.4} s, first audible sample at {tone_s:.4} s");
    assert!((white_s - 1.0).abs() < 1e-3, "white at {white_s}");
    assert!(
        (tone_s - white_s).abs() < 1.0 / 30.0,
        "A/V offset {:.4} s",
        tone_s - white_s
    );
}

#[test]
fn a_60_fps_project_exports_at_30_fps_on_the_30_fps_grid() {
    let _serial = serial();
    let gpu = warp();
    let dir = TempDir::new("rate");
    let numbered = asset_at(&fixture("numbered-gop.mp4"));
    let mut project = Project::new("Rate");
    project.settings.width = 320;
    project.settings.height = 160;
    project.settings.frame_rate = FrameRate::FPS_60;
    project.tracks[0].clips = vec![media_clip(&numbered, Time::ZERO, ms(2000))];
    project.media = vec![numbered];
    project.validate().unwrap();
    let target = dir.file("rate.mp4");
    let mut export_settings = settings(ExportQuality::High);
    export_settings.frame_rate = Some(FrameRate::FPS_30);
    let plan = plan_video_export(&project, &export_settings).unwrap();
    assert_eq!((plan.frames, plan.frame_rate), (60, FrameRate::FPS_30));
    let req = ExportRequest {
        project: Arc::new(project.clone()),
        plan,
        candidates: vec![openh264()],
        explicit_encoder: false,
        output: target.clone(),
        overwrite: false,
    };
    run_export(Arc::clone(&gpu), req, &Observer::default()).expect("the export");
    let (decoded, time_base) = decode(&target, 0);
    assert_eq!(decoded.len(), 60);
    let step = decoded[1].pts - decoded[0].pts;
    assert_eq!(
        i64::from(time_base.den) / (step * i64::from(time_base.num)),
        30
    );
    // Each exported frame is the project at n / 30 s: every second frame of the 60 fps grid.
    let mut at_30 = project;
    at_30.settings.frame_rate = FrameRate::FPS_30;
    let reference = reference_planes(&gpu, &at_30, 60);
    let y = psnr(
        decoded
            .iter()
            .map(decoded_planes)
            .zip(&reference)
            .flat_map(|(a, r)| a.y.into_iter().zip(r.y.clone())),
    );
    println!("60 -> 30 fps: PSNR-Y {y:.2} dB");
    assert!(y >= 40.0, "PSNR-Y {y:.2} dB");
}

/// A short project of colour clips: `seconds` long, 30 fps, 320 x 180.
fn colour_project(seconds: i64) -> Project {
    let mut project = Project::new("Colours");
    project.settings.width = 320;
    project.settings.height = 180;
    project.settings.frame_rate = FrameRate::FPS_30;
    project.tracks[0].clips = (0..seconds)
        .map(|s| {
            let v = (s * 40 % 256) as u8;
            Clip::color("C", ms(s * 1000), ms(1000), Rgba8::rgb(v, 255 - v, 0x80))
        })
        .collect();
    project.validate().unwrap();
    project
}

#[test]
fn cancel_stops_within_500_ms_and_leaves_no_file() {
    let _serial = serial();
    let dir = TempDir::new("cancel");
    let target = dir.file("cancelled.mp4");
    let obs = Observer::cancelling_after(10);
    let result = run_export(
        warp(),
        request(colour_project(10), ExportQuality::Recommended, target),
        &obs,
    );
    let returned = Instant::now();
    assert_eq!(result, Err(ExportError::Cancelled));
    let cancelled_at = obs.cancelled_at.lock().unwrap().expect("cancel was seen");
    let latency = returned - cancelled_at;
    println!("cancel to return: {latency:?}");
    assert!(latency < Duration::from_millis(500), "{latency:?}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_media_file_deleted_before_the_export_fails_with_its_id() {
    let _serial = serial();
    let dir = TempDir::new("offline");
    let copy = dir.file("clip-copy.mp4");
    std::fs::copy(fixture("h264-aac.mp4"), &copy).unwrap();
    let clip = asset_at(&copy);
    let id = clip.id;
    let mut project = Project::new("Offline");
    project.settings.width = 160;
    project.settings.height = 90;
    project.tracks[0].clips = vec![media_clip(&clip, Time::ZERO, ms(1000))];
    project.media = vec![clip];
    project.validate().unwrap();
    std::fs::remove_file(&copy).unwrap();
    let failure = failed(run_export(
        warp(),
        request(project, ExportQuality::Low, dir.file("offline.mp4")),
        &Observer::default(),
    ));
    assert_eq!(failure.kind, ExportErrorKind::MediaOffline);
    assert_eq!(failure.media, Some(id));
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

/// OpenH264 under another kind's name that fails on the `fail_at`-th `send_frame` (0-based).
struct Failing {
    inner: Box<dyn VideoEncoder>,
    kind: EncoderKind,
    fail_at: usize,
    sent: usize,
}

impl VideoEncoder for Failing {
    fn candidate(&self) -> EncoderCandidate {
        EncoderCandidate {
            kind: self.kind,
            ..self.inner.candidate()
        }
    }

    fn context(&self) -> &AVCodecContext {
        self.inner.context()
    }

    fn pixel_format(&self) -> ffi::AVPixelFormat {
        self.inner.pixel_format()
    }

    fn send_frame(&mut self, frame: Option<&AVFrame>) -> Result<(), RsmpegError> {
        if self.sent == self.fail_at {
            return Err(RsmpegError::AVError(ffi::AVERROR_EXTERNAL));
        }
        self.sent += 1;
        self.inner.send_frame(frame)
    }

    fn receive_packet(&mut self) -> Result<AVPacket, RsmpegError> {
        self.inner.receive_packet()
    }
}

/// Opens test candidates: NVENC fails to open, Quick Sync fails on its first frame, AMF on
/// its sixth (after packets were muxed), everything else is the real encoder. Records which
/// partial files existed at each open.
fn fake_open(
    partial: PathBuf,
    seen: Arc<Mutex<Vec<bool>>>,
) -> impl Fn(EncoderCandidate, &EncoderSettings) -> Result<Box<dyn VideoEncoder>, EncoderError> {
    move |candidate, settings| {
        seen.lock().unwrap().push(partial.exists());
        let real = |kind| {
            open_encoder(EncoderCandidate { kind, ..candidate }, settings)
                .map(|e| Box::new(e) as Box<dyn VideoEncoder>)
        };
        let failing = |fail_at| -> Result<Box<dyn VideoEncoder>, EncoderError> {
            Ok(Box::new(Failing {
                inner: real(EncoderKind::OpenH264)?,
                kind: candidate.kind,
                fail_at,
                sent: 0,
            }))
        };
        match candidate.kind {
            EncoderKind::Nvenc => Err(EncoderError::Failed {
                encoder: "fake",
                stage: "open",
                detail: "test".into(),
            }),
            EncoderKind::Qsv => failing(0),
            EncoderKind::Amf => failing(5),
            kind => real(kind),
        }
    }
}

fn candidate(kind: EncoderKind) -> EncoderCandidate {
    EncoderCandidate { kind, ..openh264() }
}

#[test]
fn encoders_that_fail_before_the_first_packet_fall_back_to_the_next_one() {
    let _serial = serial();
    let dir = TempDir::new("fallback");
    let target = dir.file("fallback.mp4");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = fake_open(partial_path(&target), Arc::clone(&seen));
    let mut req = request(
        colour_project(2),
        ExportQuality::Recommended,
        target.clone(),
    );
    req.candidates = vec![
        candidate(EncoderKind::Nvenc),
        candidate(EncoderKind::Qsv),
        openh264(),
    ];
    let obs = Observer::default();
    let summary = run_export_with(
        warp(),
        req,
        &obs,
        ExportHooks {
            open_video: Some(&open),
            ..ExportHooks::default()
        },
    )
    .expect("the export falls back to OpenH264");
    assert_eq!(summary.encoder, EncoderKind::OpenH264);
    // No partial file was left over when each candidate was opened.
    assert_eq!(*seen.lock().unwrap(), [false, false, false]);
    assert_eq!(obs.done.load(Ordering::SeqCst), 60, "frames counted once");
    assert_eq!(dir.listing(), ["fallback.mp4"]);
    let (frames, _) = decode(&target, 0);
    assert_eq!(frames.len(), 60);
}

#[test]
fn an_encoder_error_after_the_first_packet_fails_the_export() {
    let _serial = serial();
    let dir = TempDir::new("encoder-failed");
    let target = dir.file("failed.mp4");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = fake_open(partial_path(&target), seen);
    let mut req = request(colour_project(2), ExportQuality::Recommended, target);
    req.candidates = vec![candidate(EncoderKind::Amf), openh264()];
    let hooks = ExportHooks {
        open_video: Some(&open),
        ..ExportHooks::default()
    };
    let failure = failed(run_export_with(warp(), req, &Observer::default(), hooks));
    assert_eq!(failure.kind, ExportErrorKind::EncoderFailed, "{failure:?}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn an_explicit_encoder_does_not_fall_back() {
    let _serial = serial();
    let dir = TempDir::new("explicit");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let target = dir.file("explicit.mp4");
    let open = fake_open(partial_path(&target), seen);
    let mut req = request(colour_project(1), ExportQuality::Low, target);
    req.candidates = vec![candidate(EncoderKind::Qsv)];
    req.explicit_encoder = true;
    let hooks = ExportHooks {
        open_video: Some(&open),
        ..ExportHooks::default()
    };
    let failure = failed(run_export_with(warp(), req, &Observer::default(), hooks));
    assert_eq!(failure.kind, ExportErrorKind::EncoderFailed);
    assert!(dir.listing().is_empty());

    let failure = failure_of_empty_candidates();
    assert_eq!(failure.kind, ExportErrorKind::NoEncoder);
}

fn failure_of_empty_candidates() -> engine::export::run::ExportFailure {
    let dir = TempDir::new("no-encoder");
    let mut req = request(colour_project(1), ExportQuality::Low, dir.file("none.mp4"));
    req.candidates.clear();
    failed(run_export(warp(), req, &Observer::default()))
}

#[test]
fn a_full_disk_fails_the_export_and_deletes_the_partial_file() {
    let _serial = serial();
    let dir = TempDir::new("disk-full");
    // ENOSPC from the writer after 3 kB of the ~7 kB file (faststart off, export.md §11).
    let req = request(colour_project(3), ExportQuality::High, dir.file("full.mp4"));
    let failure = failed(run_export_with(
        warp(),
        req,
        &Observer::default(),
        ExportHooks {
            write_limit: Some(3_000),
            ..ExportHooks::default()
        },
    ));
    assert_eq!(failure.kind, ExportErrorKind::DiskFull, "{failure:?}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());

    // The free-space floor: 1 MiB free is below 64 MiB.
    let tiny = |_: &Path| Some(1024 * 1024);
    let req = request(colour_project(1), ExportQuality::Low, dir.file("floor.mp4"));
    let failure = failed(run_export_with(
        warp(),
        req,
        &Observer::default(),
        ExportHooks {
            free_space: Some(&tiny),
            ..ExportHooks::default()
        },
    ));
    assert_eq!(failure.kind, ExportErrorKind::DiskFull);
    assert!(dir.listing().is_empty());
}

#[test]
fn an_existing_target_is_replaced_only_with_overwrite_and_a_stale_partial_is_removed() {
    let _serial = serial();
    let dir = TempDir::new("overwrite");
    let target = dir.file("clip.mp4");
    std::fs::write(&target, b"old").unwrap();
    // A partial file left by a crash.
    std::fs::write(partial_path(&target), b"crash").unwrap();

    let failure = failed(run_export(
        warp(),
        request(colour_project(1), ExportQuality::Low, target.clone()),
        &Observer::default(),
    ));
    assert_eq!(failure.kind, ExportErrorKind::WriteFailed);
    let kept = failure.kept_path.expect("the finished file is kept");
    assert_eq!(kept, partial_path(&target));
    assert_eq!(std::fs::read(&target).unwrap(), b"old", "not replaced");
    std::fs::remove_file(&kept).unwrap();

    let mut req = request(colour_project(1), ExportQuality::Low, target.clone());
    req.overwrite = true;
    let summary = run_export(warp(), req, &Observer::default()).expect("the export");
    assert!(summary.bytes > 1000);
    assert_eq!(dir.listing(), ["clip.mp4"]);
}

#[test]
fn playback_throttles_the_export_to_20_frames_per_second() {
    let _serial = serial();
    let dir = TempDir::new("throttle");
    let obs = Observer::default();
    obs.playing.store(true, Ordering::SeqCst);
    let started = Instant::now();
    run_export(
        warp(),
        request(colour_project(1), ExportQuality::Low, dir.file("t.mp4")),
        &obs,
    )
    .expect("the export");
    let elapsed = started.elapsed();
    // 30 frames, each starting at least 50 ms after the previous one.
    println!("30 frames while playing: {elapsed:?}");
    assert!(elapsed >= Duration::from_millis(29 * 50), "{elapsed:?}");
}

/// A 1-minute 1080p30 export of colour clips with an image overlay through the encoders of
/// `CUTFRAME_ENCODERS` (default OpenH264) on the adapter of `CUTFRAME_GPU`; with
/// `CUTFRAME_EXPORT_BENCH_SOURCE=<video>` the main track is that video (cut every 3 s), and
/// `CUTFRAME_EXPORT_BENCH_SIZE=4k` exports 2160p. Prints the frame rate (export.md §10).
#[test]
#[ignore = "budget run on real hardware (export.md §10); release build recommended"]
fn export_budget() {
    use engine::export::encoders::{EncoderFilter, ProbeReport};
    let _serial = serial();
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_test_writer()
        .try_init();
    let gpu = Arc::new(GpuContext::new().expect("a GPU"));
    println!("GPU: {:?}", gpu.info());
    let filter = EncoderFilter::from_env();
    let report = ProbeReport::run(&filter);
    let candidates = report.candidates(VideoCodec::H264, None);
    println!(
        "candidates: {:?}",
        candidates.iter().map(|c| c.kind).collect::<Vec<_>>()
    );
    let four_k = std::env::var("CUTFRAME_EXPORT_BENCH_SIZE").is_ok_and(|v| v == "4k");
    let (w, h) = if four_k { (3840, 2160) } else { (1920, 1080) };
    let seconds: i64 = std::env::var("CUTFRAME_EXPORT_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut project = Project::new("Bench");
    project.settings.width = w;
    project.settings.height = h;
    project.settings.frame_rate = FrameRate::FPS_30;
    let image = asset_at(&fixture("image-alpha.png"));
    let sine = asset_at(&fixture("sine-1k-48k.flac"));
    if let Some(source) = std::env::var_os("CUTFRAME_EXPORT_BENCH_SOURCE") {
        let video = asset_at(Path::new(&source));
        let len = video
            .info
            .duration
            .expect("a video with a duration")
            .flicks();
        let cut = 3 * FLICKS_PER_SECOND;
        project.tracks[0].clips = (0..seconds / 3)
            .map(|i| Clip {
                source: ClipSource::media_at(
                    video.id,
                    Time((i * 7 * FLICKS_PER_SECOND) % (len - cut).max(1)),
                ),
                ..Clip::color("V", Time(i * cut), Time(cut), Rgba8::BLACK)
            })
            .collect();
        project.media.push(video);
    } else {
        project.tracks[0].clips = colour_project(seconds).tracks[0].clips.clone();
    }
    project.tracks[1].clips = (0..seconds)
        .map(|s| audio_clip(&sine, ms(s * 1000), ms(1000), 0.3))
        .collect();
    let mut overlay = Track::new(TrackKind::Video, "Overlay");
    overlay.clips = vec![media_clip(&image, Time::ZERO, ms(seconds * 1000))];
    project.tracks.insert(0, overlay);
    project.media.extend([image, sine]);
    project.validate().unwrap();

    let dir = TempDir::new("bench");
    let mut req = request(project, ExportQuality::Recommended, dir.file("bench.mp4"));
    req.candidates = candidates;
    let frames = req.plan.frames;
    let started = Instant::now();
    let summary = run_export(gpu, req, &Observer::default()).expect("the export");
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "{w}x{h}: {frames} frames in {elapsed:.2} s = {:.1} frames/s ({:.2}x real time), {:?}, {} bytes",
        f64::from(frames) / elapsed,
        f64::from(frames) / 30.0 / elapsed,
        summary.encoder,
        summary.bytes
    );
}
