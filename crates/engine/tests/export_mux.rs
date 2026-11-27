//! Export audio and MP4 muxing (`engine::export::{audio, mux}`, export.md §4.5, §4.6, §11
//! "Container checks"): tiny exports of test projects through the same steps as the export loop
//! (offline render on WARP → NV12 pass → OpenH264, `mix_block` → AAC, interleaved MP4), then the
//! file is demuxed and decoded with FFmpeg:
//!
//! - `moov` before `mdat` (`+faststart`);
//! - one H.264 High video stream with `colr` nclx BT.709 / limited and the same in the bitstream
//!   VUI, `frames` packets with a constant `pts` step (CFR);
//! - one AAC-LC stereo stream at the project rate whose decoded length (after the edit list) is
//!   the plan's sample count within one AAC frame, and whose sine peaks at the clip volume;
//! - video and audio start together;
//! - silent timelines get an AAC track of silence.
//!
//! OpenH264 (`libopenh264`, in the pinned build) is the encoder because it runs everywhere,
//! including CI. The HEVC `hvc1` check needs a hardware encoder and is ignored by default.
//! Output files go to the temp directory and are deleted; with `CUTFRAME_EXPORT_TEST_KEEP=<dir>`
//! a copy of each is kept in `<dir>` for a look with `ffprobe`.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_core::media::MediaPath;
use app_core::model::ClipId;
use app_core::{
    Animated, AudioProps, Clip, ClipSource, FLICKS_PER_SECOND, FrameRate, MediaAsset, MediaId,
    Project, Rgba8, Time,
};
use engine::export::audio::AudioEncoder;
use engine::export::mux::{Muxer, StreamError, apply_color_tags, tag_frame};
use engine::export::nv12::{Nv12Frame, Nv12Pass, YuvLayout, alloc_video_frame};
use engine::media::{fingerprint, probe};
use engine::render::{
    AdapterMode, GpuContext, OfflineOptions, OfflineRenderer, OutputFormat, RenderedFrame,
};
use rsmpeg::UnsafeDerefMut;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::{AVDictionary, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

fn warp() -> Arc<GpuContext> {
    Arc::new(
        GpuContext::new_with(AdapterMode::SoftwareOnly)
            .expect("WARP (the D3D12 software adapter) ships with Windows"),
    )
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

/// A file in the temp directory, deleted on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "cutframe-export-test-{}-{name}.cutframe-partial.mp4",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let (Some(dir), Some(name)) = (
            std::env::var_os("CUTFRAME_EXPORT_TEST_KEEP"),
            self.0.file_name(),
        ) {
            let _ = std::fs::copy(&self.0, Path::new(&dir).join(name));
        }
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The plan's counts (export.md §4.1): `frames = ceil(D / F)`, `samples = floor(frames x F x
/// rate / FLICKS_PER_SECOND)`.
fn counts(project: &Project) -> (u32, u64) {
    let duration = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(Clip::end)
        .max()
        .unwrap_or(Time::ZERO);
    let frame = project.settings.frame_rate.frame_duration().flicks();
    let frames = duration.flicks().div_euclid(frame) + i64::from(duration.flicks() % frame != 0);
    let samples = i128::from(frames) * i128::from(frame) * i128::from(project.settings.sample_rate)
        / i128::from(FLICKS_PER_SECOND);
    (frames as u32, samples as u64)
}

/// `libopenh264` as the export opens it (export.md §4.4 `Full` option set, colour tags).
fn openh264(size: (u32, u32), rate: FrameRate) -> AVCodecContext {
    let codec = AVCodec::find_encoder_by_name(c"libopenh264").expect("libopenh264 in the build");
    let mut context = AVCodecContext::new(&codec);
    context.set_width(size.0 as i32);
    context.set_height(size.1 as i32);
    context.set_pix_fmt(ffi::AV_PIX_FMT_YUV420P);
    let (num, den) = (rate.num() as i32, rate.den() as i32);
    context.set_time_base(ffi::AVRational { num: den, den: num });
    context.set_framerate(ffi::AVRational { num, den });
    context.set_bit_rate(2_000_000);
    context.set_gop_size(((num + den / 2) / den / 2).max(1));
    context.set_max_b_frames(0);
    context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    apply_color_tags(&mut context);
    let options = AVDictionary::new(c"rc_mode", c"bitrate", 0)
        .set(c"profile", c"high", 0)
        .set(c"coder", c"cabac", 0)
        .set(c"allow_skip_frames", c"0", 0);
    context.open(Some(options)).expect("open libopenh264");
    context
}

fn drain_video(context: &mut AVCodecContext, muxer: &mut Muxer) {
    loop {
        match context.receive_packet() {
            Ok(packet) => muxer.write_video(packet).expect("write video"),
            Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => return,
            Err(err) => panic!("receive_packet: {err}"),
        }
    }
}

/// Exports `project` to `path` the way the export loop does (export.md §4.2): render frame `n`,
/// NV12 pass into the ring, copy every ready slot into the encoder frame (`pts = k`), send,
/// mux, then audio up to `(k + 1) x F`; finally drain, flush both encoders and write the trailer.
fn export(
    gpu: &Arc<GpuContext>,
    project: &Arc<Project>,
    mut video: AVCodecContext,
    layout: YuvLayout,
    path: &Path,
) {
    let size = (project.settings.width, project.settings.height);
    let rate = project.settings.frame_rate;
    let (frames, samples) = counts(project);
    let mut renderer = OfflineRenderer::new(Arc::clone(gpu), OfflineOptions::default());
    let mut pass = Nv12Pass::new(Arc::clone(gpu), size);
    let mut audio =
        AudioEncoder::new(Arc::clone(project), project.settings.sample_rate, samples).unwrap();
    let mut muxer = Muxer::create(path, &video, audio.context()).expect("create the muxer");
    let mut picture = alloc_video_frame(size, layout).unwrap();
    tag_frame(&mut picture);

    let mut hand_over = |frame: Nv12Frame<'_>,
                         video: &mut AVCodecContext,
                         audio: &mut AudioEncoder,
                         muxer: &mut Muxer| {
        let k = frame.index();
        frame.copy_to_avframe(&mut picture).unwrap();
        drop(frame);
        picture.set_pts(k as i64);
        video.send_frame(Some(&picture)).expect("send_frame");
        drain_video(video, muxer);
        audio
            .encode_until(Time::from_frames(k as i64 + 1, rate), &mut |packet| {
                muxer.write_audio(packet)
            })
            .unwrap();
    };
    for n in 0..frames {
        let canvas =
            match renderer.render_frame(project, i64::from(n), size, OutputFormat::GpuCanvas) {
                Ok(RenderedFrame::Gpu(view)) => view,
                other => panic!("frame {n}: {other:?}"),
            };
        if pass.is_full() {
            let frame = pass.next_ready(true).unwrap().unwrap();
            hand_over(frame, &mut video, &mut audio, &mut muxer);
        }
        pass.submit(canvas, u64::from(n)).unwrap();
        while let Some(frame) = pass.next_ready(false).unwrap() {
            hand_over(frame, &mut video, &mut audio, &mut muxer);
        }
    }
    while let Some(frame) = pass.next_ready(true).unwrap() {
        hand_over(frame, &mut video, &mut audio, &mut muxer);
    }
    video.send_frame(None).expect("flush video");
    drain_video(&mut video, &mut muxer);
    audio
        .finish(&mut |packet| muxer.write_audio(packet))
        .unwrap();
    assert_eq!(audio.encoded_samples(), samples);
    assert_eq!(muxer.video_packets(), u64::from(frames));
    assert_eq!(muxer.finish().unwrap(), path);
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

#[derive(Debug)]
struct StreamFacts {
    codec_id: ffi::AVCodecID,
    codec_tag: u32,
    profile: i32,
    colour: [u32; 5],
    sample_rate: i32,
    channels: i32,
    time_base: ffi::AVRational,
    start: f64,
    duration: f64,
    pts: Vec<i64>,
}

fn seconds(ticks: i64, tb: ffi::AVRational) -> f64 {
    ticks as f64 * f64::from(tb.num) / f64::from(tb.den)
}

/// The facts of every stream of `path`, plus the packets read for each.
fn demux(path: &Path) -> Vec<StreamFacts> {
    let name = CString::new(path.to_str().unwrap()).unwrap();
    let mut input = AVFormatContextInput::open(&name).expect("open the export");
    let mut facts: Vec<StreamFacts> = input
        .streams()
        .iter()
        .map(|stream| {
            let par = stream.codecpar();
            StreamFacts {
                codec_id: par.codec_id,
                codec_tag: par.codec_tag,
                profile: par.profile,
                colour: [
                    par.color_primaries as u32,
                    par.color_trc as u32,
                    par.color_space as u32,
                    par.color_range as u32,
                    par.chroma_location as u32,
                ],
                sample_rate: par.sample_rate,
                channels: par.ch_layout.nb_channels,
                time_base: stream.time_base,
                start: seconds(stream.start_time, stream.time_base),
                duration: seconds(stream.duration, stream.time_base),
                pts: Vec::new(),
            }
        })
        .collect();
    while let Some(packet) = input.read_packet().unwrap() {
        facts[packet.stream_index as usize].pts.push(packet.pts);
    }
    facts
}

/// Decodes stream `index` of `path` with the colour fields of the container cleared, so the
/// decoder's frames carry what the bitstream says. Returns the decoded frames.
fn decode(path: &Path, index: usize) -> Vec<AVFrame> {
    let name = CString::new(path.to_str().unwrap()).unwrap();
    let mut input = AVFormatContextInput::open(&name).unwrap();
    let mut decoder = {
        let stream = &input.streams()[index];
        let par = stream.codecpar();
        let codec = AVCodec::find_decoder(par.codec_id).expect("a decoder");
        let mut decoder = AVCodecContext::new(&codec);
        decoder.apply_codecpar(&par).unwrap();
        decoder.set_pkt_timebase(stream.time_base);
        decoder
    };
    {
        // SAFETY: plain field writes on a valid, unopened codec context.
        let raw = unsafe { decoder.deref_mut() };
        raw.color_primaries = ffi::AVCOL_PRI_UNSPECIFIED;
        raw.color_trc = ffi::AVCOL_TRC_UNSPECIFIED;
        raw.colorspace = ffi::AVCOL_SPC_UNSPECIFIED;
        raw.color_range = ffi::AVCOL_RANGE_UNSPECIFIED;
    }
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
    frames
}

/// Left-channel samples of decoded `fltp` audio frames.
fn left_samples(frames: &[AVFrame]) -> Vec<f32> {
    let mut out = Vec::new();
    for frame in frames {
        assert_eq!(frame.format, ffi::AV_SAMPLE_FMT_FLTP);
        // SAFETY: a decoded `fltp` frame holds `nb_samples` `f32`s in `data[0]`.
        let plane = unsafe {
            std::slice::from_raw_parts(frame.data[0].cast::<f32>(), frame.nb_samples as usize)
        };
        out.extend_from_slice(plane);
    }
    out
}

const BT709_LIMITED_LEFT: [u32; 5] = [
    ffi::AVCOL_PRI_BT709 as u32,
    ffi::AVCOL_TRC_BT709 as u32,
    ffi::AVCOL_SPC_BT709 as u32,
    ffi::AVCOL_RANGE_MPEG as u32,
    ffi::AVCHROMA_LOC_LEFT as u32,
];

/// A 160 x 90, 30 fps project of 1 s: two colour clips, and the 1 kHz sine at volume 0.5 for
/// the whole second.
fn sine_project() -> Project {
    let sine = asset_at(&fixture("sine-1k-48k.flac"));
    let mut project = Project::new("export");
    project.settings.width = 160;
    project.settings.height = 90;
    project.settings.frame_rate = FrameRate::FPS_30;
    let half = Time(FLICKS_PER_SECOND / 2);
    project.tracks[0].clips = vec![
        Clip::color("Red", Time::ZERO, half, Rgba8::rgb(0xc0, 0x20, 0x20)),
        Clip::color("Blue", half, half, Rgba8::rgb(0x20, 0x40, 0xc0)),
    ];
    project.tracks[1].clips = vec![Clip {
        id: ClipId::new(),
        name: "Sine".into(),
        start: Time::ZERO,
        duration: Time(FLICKS_PER_SECOND),
        source: ClipSource::media_at(sine.id, Time::ZERO),
        link: None,
        visual: None,
        audio: Some(AudioProps {
            volume: Animated::constant(0.5),
            ..AudioProps::default()
        }),
    }];
    project.media.push(sine);
    project.validate().expect("a valid project");
    project
}

#[test]
fn an_openh264_export_is_a_faststart_cfr_mp4_with_tagged_colour_and_aac_audio() {
    let gpu = warp();
    let project = Arc::new(sine_project());
    let (frames, samples) = counts(&project);
    assert_eq!((frames, samples), (30, 48_000));
    let file = TempFile::new("openh264");
    let size = (project.settings.width, project.settings.height);
    export(
        &gpu,
        &project,
        openh264(size, project.settings.frame_rate),
        YuvLayout::Yuv420p,
        &file.0,
    );

    let boxes = top_level_boxes(&file.0);
    println!("boxes: {boxes:?}");
    let moov = boxes.iter().position(|b| b == "moov").expect("moov");
    let mdat = boxes.iter().position(|b| b == "mdat").expect("mdat");
    assert!(moov < mdat, "faststart: moov before mdat, got {boxes:?}");

    let streams = demux(&file.0);
    assert_eq!(streams.len(), 2, "{streams:?}");
    let (video, audio) = (&streams[0], &streams[1]);
    println!("video: {video:?}");
    println!(
        "audio: codec {} profile {} {} Hz {} ch, start {:.6} s, duration {:.6} s, {} packets",
        audio.codec_id,
        audio.profile,
        audio.sample_rate,
        audio.channels,
        audio.start,
        audio.duration,
        audio.pts.len()
    );

    // Video: H.264 High, colr nclx BT.709 limited, CFR, `frames` frames.
    assert_eq!(video.codec_id, ffi::AV_CODEC_ID_H264);
    assert_eq!(
        video.profile,
        ffi::AV_PROFILE_H264_HIGH as i32,
        "H.264 High"
    );
    assert_eq!(
        video.colour[..4],
        BT709_LIMITED_LEFT[..4],
        "container colour tags"
    );
    assert_eq!(video.pts.len(), frames as usize, "one packet per frame");
    let mut pts = video.pts.clone();
    pts.sort_unstable();
    let step = pts[1] - pts[0];
    assert!(
        step > 0 && pts.windows(2).all(|w| w[1] - w[0] == step),
        "CFR: {pts:?}"
    );
    let frame_s = 1.0 / 30.0;
    assert!((seconds(step, video.time_base) - frame_s).abs() < 1e-6);
    assert!(
        (video.duration - 1.0).abs() < 1e-6,
        "video lasts {} s",
        video.duration
    );

    // The bitstream VUI says the same as the container.
    let decoded = decode(&file.0, 0);
    assert_eq!(decoded.len(), frames as usize);
    let first = &decoded[0];
    assert_eq!(
        [
            first.color_primaries as u32,
            first.color_trc as u32,
            first.colorspace as u32,
            first.color_range as u32,
        ],
        BT709_LIMITED_LEFT[..4],
        "VUI colour"
    );

    // Audio: AAC-LC stereo at the project rate, as long as the video within one AAC frame,
    // starting with it.
    assert_eq!(audio.codec_id, ffi::AV_CODEC_ID_AAC);
    assert_eq!(audio.profile, ffi::AV_PROFILE_AAC_LOW as i32);
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
    let aac_frame_s = 1024.0 / 48_000.0;
    assert!(
        (audio.duration - video.duration).abs() <= aac_frame_s,
        "audio {} s, video {} s",
        audio.duration,
        video.duration
    );
    assert!(
        (audio.start - video.start).abs() < frame_s,
        "A/V start {} vs {}",
        audio.start,
        video.start
    );
    let pcm = left_samples(&decode(&file.0, 1));
    println!("decoded audio: {} samples (plan {samples})", pcm.len());
    assert!(
        pcm.len() as u64 >= samples && pcm.len() as u64 <= samples + 1024,
        "{} decoded samples",
        pcm.len()
    );
    let peak = pcm[2_000..46_000]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    println!("sine peak {peak}");
    assert!((peak - 0.5).abs() <= 0.02, "peak {peak}");
}

#[test]
fn a_silent_timeline_at_44_1_khz_still_gets_an_aac_track_of_exact_length() {
    let gpu = warp();
    let mut project = Project::new("silent");
    project.settings.width = 64;
    project.settings.height = 36;
    project.settings.frame_rate = FrameRate::FPS_25;
    project.settings.sample_rate = 44_100;
    // 1.01 s: 26 frames at 25 fps, so the audio ends with the last frame at 1.04 s.
    project.tracks[0].clips = vec![Clip::color(
        "Grey",
        Time::ZERO,
        Time(FLICKS_PER_SECOND * 101 / 100),
        Rgba8::rgb(0x80, 0x80, 0x80),
    )];
    project.validate().expect("a valid project");
    let project = Arc::new(project);
    let (frames, samples) = counts(&project);
    assert_eq!((frames, samples), (26, 45_864));
    let file = TempFile::new("silent");
    export(
        &gpu,
        &project,
        openh264((64, 36), FrameRate::FPS_25),
        YuvLayout::Yuv420p,
        &file.0,
    );
    let streams = demux(&file.0);
    assert_eq!(streams.len(), 2);
    let (video, audio) = (&streams[0], &streams[1]);
    assert_eq!(video.pts.len(), 26);
    assert_eq!(
        (audio.codec_id, audio.sample_rate),
        (ffi::AV_CODEC_ID_AAC, 44_100)
    );
    assert!((audio.duration - video.duration).abs() <= 1024.0 / 44_100.0);
    let pcm = left_samples(&decode(&file.0, 1));
    println!(
        "silent: {} decoded samples (plan {samples}), video {} s, audio {} s",
        pcm.len(),
        video.duration,
        audio.duration
    );
    assert!(pcm.len() as u64 >= samples && pcm.len() as u64 <= samples + 1024);
    assert!(pcm.iter().all(|s| s.abs() < 1e-4), "silence");
}

#[test]
fn a_missing_folder_is_a_write_error() {
    let project = Arc::new(sine_project());
    let video = openh264((160, 90), FrameRate::FPS_30);
    let audio = AudioEncoder::new(Arc::clone(&project), 48_000, 48_000).unwrap();
    let path = std::env::temp_dir()
        .join("cutframe-export-test-no-such-folder")
        .join("out.mp4");
    match Muxer::create(&path, &video, audio.context()) {
        Err(StreamError::WriteFailed(detail)) => println!("{detail}"),
        other => panic!("{other:?}"),
    }
}

/// A hardware HEVC encoder that opens at `size` with NV12 input, if the machine has one.
fn hardware_hevc(size: (u32, u32)) -> Option<(&'static str, AVCodecContext)> {
    for name in [c"hevc_nvenc", c"hevc_qsv", c"hevc_amf", c"hevc_mf"] {
        let Some(codec) = AVCodec::find_encoder_by_name(name) else {
            continue;
        };
        let mut context = AVCodecContext::new(&codec);
        context.set_width(size.0 as i32);
        context.set_height(size.1 as i32);
        context.set_pix_fmt(ffi::AV_PIX_FMT_NV12);
        context.set_time_base(ffi::AVRational { num: 1, den: 30 });
        context.set_framerate(ffi::AVRational { num: 30, den: 1 });
        context.set_bit_rate(4_000_000);
        context.set_gop_size(15);
        context.set_max_b_frames(0);
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
        apply_color_tags(&mut context);
        let options = (name == c"hevc_mf").then(|| AVDictionary::new(c"hw_encoding", c"1", 0));
        match context.open(options) {
            Ok(_) => return Some((name.to_str().unwrap(), context)),
            Err(err) => println!("{name:?} does not open: {err}"),
        }
    }
    None
}

#[test]
#[ignore = "needs a hardware HEVC encoder (NVENC, Quick Sync, AMF or a Media Foundation MFT)"]
fn a_hardware_hevc_export_is_tagged_hvc1() {
    let gpu = warp();
    let mut project = sine_project();
    project.settings.width = 640;
    project.settings.height = 360;
    let project = Arc::new(project);
    let Some((name, video)) = hardware_hevc((640, 360)) else {
        println!("no hardware HEVC encoder on this machine");
        return;
    };
    println!("HEVC encoder: {name}");
    let file = TempFile::new("hevc");
    export(&gpu, &project, video, YuvLayout::Nv12, &file.0);
    let streams = demux(&file.0);
    let video = &streams[0];
    assert_eq!(video.codec_id, ffi::AV_CODEC_ID_HEVC);
    assert_eq!(
        video.codec_tag,
        u32::from_le_bytes(*b"hvc1"),
        "hvc1, not hev1"
    );
    assert_eq!(video.colour[..4], BT709_LIMITED_LEFT[..4]);
    let boxes = top_level_boxes(&file.0);
    assert!(
        boxes.iter().position(|b| b == "moov") < boxes.iter().position(|b| b == "mdat"),
        "{boxes:?}"
    );
}
