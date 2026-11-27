//! Video encoders for export (export.md §4.4): the candidate table, the option sets, opening an
//! encoder at given settings, the session probe (real test encodes) and `CUTFRAME_ENCODERS`.
//!
//! Every encoder here is in the pinned LGPL FFmpeg build: GPU encoders (NVENC, Quick Sync, AMF),
//! Media Foundation (a hardware MFT, or Microsoft's software H.264 encoder) and OpenH264 as the
//! last resort (decision F28: kept for private use). Hardware encoders are compiled in whether
//! or not the machine has the GPU, so only a test encode says what works: [`ProbeReport::run`]
//! opens each candidate at 1280 x 720 and turns 8 frames into packets, once per session.
//!
//! Frames go to every encoder in system memory (NV12, or YUV 4:2:0 planar for OpenH264); the
//! encoders upload them to their own device themselves, so no candidate needs the render device
//! (zero-copy is task 0700).

use std::ffi::CStr;
use std::time::{Duration, Instant};

use app_core::FrameRate;
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVCodecRef};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::plan::{ExportPlan, VideoCodec};

/// Which encoder implementation. Keys (`camelCase`) are used by `CUTFRAME_ENCODERS` and IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncoderKind {
    Nvenc,
    Qsv,
    Amf,
    MfHardware,
    MfSoftware,
    OpenH264,
}

impl EncoderKind {
    pub const ALL: [EncoderKind; 6] = [
        Self::Nvenc,
        Self::Qsv,
        Self::Amf,
        Self::MfHardware,
        Self::MfSoftware,
        Self::OpenH264,
    ];

    /// The stable key, as in IPC and `CUTFRAME_ENCODERS`.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::Qsv => "qsv",
            Self::Amf => "amf",
            Self::MfHardware => "mfHardware",
            Self::MfSoftware => "mfSoftware",
            Self::OpenH264 => "openH264",
        }
    }

    /// Runs on a GPU's media engine (the Media Foundation hardware MFT included).
    pub const fn is_hardware(self) -> bool {
        matches!(self, Self::Nvenc | Self::Qsv | Self::Amf | Self::MfHardware)
    }

    /// The FFmpeg encoder for `codec`; `None` if this kind has none (software HEVC).
    pub const fn encoder_name(self, codec: VideoCodec) -> Option<&'static CStr> {
        use VideoCodec::{H264, Hevc};
        Some(match (self, codec) {
            (Self::Nvenc, H264) => c"h264_nvenc",
            (Self::Nvenc, Hevc) => c"hevc_nvenc",
            (Self::Qsv, H264) => c"h264_qsv",
            (Self::Qsv, Hevc) => c"hevc_qsv",
            (Self::Amf, H264) => c"h264_amf",
            (Self::Amf, Hevc) => c"hevc_amf",
            (Self::MfHardware | Self::MfSoftware, H264) => c"h264_mf",
            (Self::MfHardware, Hevc) => c"hevc_mf",
            (Self::OpenH264, H264) => c"libopenh264",
            (Self::MfSoftware | Self::OpenH264, Hevc) => return None,
        })
    }

    /// The pixel format frames are handed over in.
    pub const fn pixel_format(self) -> ffi::AVPixelFormat {
        match self {
            Self::OpenH264 => ffi::AV_PIX_FMT_YUV420P,
            _ => ffi::AV_PIX_FMT_NV12,
        }
    }
}

/// The candidates for `codec`, best first: NVENC, Quick Sync, AMF, Media Foundation hardware,
/// then (H.264 only) Microsoft's software encoder and OpenH264.
pub const fn candidates(codec: VideoCodec) -> &'static [EncoderKind] {
    use EncoderKind::*;
    match codec {
        VideoCodec::H264 => &[Nvenc, Qsv, Amf, MfHardware, MfSoftware, OpenH264],
        VideoCodec::Hevc => &[Nvenc, Qsv, Amf, MfHardware],
    }
}

/// `Full` first; `Safe` (no B-frames, no look-ahead or multipass, a faster preset) if `Full`
/// fails to open on an older GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionSet {
    Full,
    Safe,
}

/// One encoder configuration the export may try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncoderCandidate {
    pub kind: EncoderKind,
    pub codec: VideoCodec,
    pub options: OptionSet,
}

/// Size, rate and bitrate an encoder is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub video_kbps: u32,
    pub gop: u32,
}

impl EncoderSettings {
    /// What the probe opens every candidate with.
    pub const PROBE: EncoderSettings = EncoderSettings {
        width: 1280,
        height: 720,
        frame_rate: FrameRate::FPS_30,
        video_kbps: 4_000,
        gop: 15,
    };

    pub fn for_plan(plan: &ExportPlan) -> Self {
        Self {
            width: plan.width,
            height: plan.height,
            frame_rate: plan.frame_rate,
            video_kbps: plan.video_kbps,
            gop: plan.gop,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncoderError {
    #[error("{kind:?} has no {codec:?} encoder")]
    NoEncoderForCodec {
        kind: EncoderKind,
        codec: VideoCodec,
    },
    #[error("encoder {0} is not in the linked FFmpeg")]
    NotInBuild(&'static str),
    #[error("{encoder}: {stage} failed: {detail}")]
    Failed {
        encoder: &'static str,
        stage: &'static str,
        detail: String,
    },
}

/// Private options of `candidate`'s FFmpeg encoder, as `(name, value)`. Names and values were
/// checked with `ffmpeg -h encoder=<name>` of the pinned build; a test sets every one on the
/// linked FFmpeg. Bitrate, GOP, B-frames, profile for Media Foundation and colour are generic
/// codec-context fields (see [`open_encoder`]).
pub fn private_options(candidate: EncoderCandidate) -> Vec<(&'static CStr, &'static CStr)> {
    let full = candidate.options == OptionSet::Full;
    let h264 = candidate.codec == VideoCodec::H264;
    let profile = if h264 { c"high" } else { c"main" };
    match candidate.kind {
        EncoderKind::Nvenc => {
            let mut options = vec![
                (c"profile", profile),
                (c"preset", c"p5"),
                (c"tune", c"hq"),
                (c"rc", c"vbr"),
                (c"spatial-aq", c"1"),
            ];
            if full {
                options.push((c"multipass", c"qres"));
            }
            options
        }
        EncoderKind::Qsv => vec![
            (c"profile", profile),
            (c"preset", if full { c"slower" } else { c"medium" }),
        ],
        EncoderKind::Amf => {
            let mut options = vec![
                (c"profile", profile),
                (c"usage", c"transcoding"),
                (c"quality", c"quality"),
                (c"rc", c"vbr_peak"),
            ];
            if h264 {
                // `bf` is the B-picture pattern (count); HEVC AMF has no B-frames in 7.1.
                options.push((c"bf", if full { c"2" } else { c"0" }));
            }
            options
        }
        EncoderKind::MfHardware | EncoderKind::MfSoftware => vec![
            (
                c"hw_encoding",
                if candidate.kind == EncoderKind::MfHardware {
                    c"1"
                } else {
                    c"0"
                },
            ),
            (c"rate_control", c"pc_vbr"),
            (c"scenario", c"archive"),
        ],
        EncoderKind::OpenH264 => vec![
            (c"profile", if full { c"high" } else { c"main" }),
            (c"rc_mode", c"bitrate"),
            (c"coder", c"cabac"),
            (c"allow_skip_frames", c"0"),
        ],
    }
}

/// `Safe` differs from `Full` for this encoder (otherwise a failed `Full` is not retried).
fn has_safe_variant(kind: EncoderKind, codec: VideoCodec) -> bool {
    let full = EncoderCandidate {
        kind,
        codec,
        options: OptionSet::Full,
    };
    let safe = EncoderCandidate {
        options: OptionSet::Safe,
        ..full
    };
    private_options(full) != private_options(safe) || b_frames(full) != b_frames(safe)
}

/// B-frames through the generic `max_b_frames` field (AMF uses its private `bf` instead; Media
/// Foundation and OpenH264 get none).
fn b_frames(candidate: EncoderCandidate) -> i32 {
    match (candidate.kind, candidate.options) {
        (EncoderKind::Nvenc | EncoderKind::Qsv, OptionSet::Full) => 2,
        _ => 0,
    }
}

/// An encoder opened by [`open_encoder`], ready for frames of `pixel_format` with `pts = n` in
/// `time_base = 1 / fps`.
pub struct OpenEncoder {
    pub context: AVCodecContext,
    pub pixel_format: ffi::AVPixelFormat,
    pub candidate: EncoderCandidate,
}

impl std::fmt::Debug for OpenEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenEncoder")
            .field("candidate", &self.candidate)
            .finish_non_exhaustive()
    }
}

fn name_str(name: &'static CStr) -> &'static str {
    name.to_str().unwrap_or("?")
}

/// The FFmpeg encoder of `candidate`, from the linked build.
fn find(candidate: EncoderCandidate) -> Result<(AVCodecRef<'static>, &'static str), EncoderError> {
    let name =
        candidate
            .kind
            .encoder_name(candidate.codec)
            .ok_or(EncoderError::NoEncoderForCodec {
                kind: candidate.kind,
                codec: candidate.codec,
            })?;
    let codec =
        AVCodec::find_encoder_by_name(name).ok_or(EncoderError::NotInBuild(name_str(name)))?;
    Ok((codec, name_str(name)))
}

/// Sets every private option of `candidate` on `context`. An unknown name or a rejected value is
/// an error (never silently ignored, unlike an options dictionary).
fn set_private_options(
    context: &mut AVCodecContext,
    candidate: EncoderCandidate,
    encoder: &'static str,
) -> Result<(), EncoderError> {
    for (name, value) in private_options(candidate) {
        // SAFETY: `priv_data` is the encoder's private context (an `AVClass`-based struct)
        // allocated by `avcodec_alloc_context3`; both strings are NUL-terminated.
        let ret = unsafe {
            ffi::av_opt_set(
                (*context.as_mut_ptr()).priv_data,
                name.as_ptr(),
                value.as_ptr(),
                0,
            )
        };
        if ret < 0 {
            return Err(EncoderError::Failed {
                encoder,
                stage: "option",
                detail: format!(
                    "{}={}: {}",
                    name_str(name),
                    name_str(value),
                    RsmpegError::AVError(ret)
                ),
            });
        }
    }
    Ok(())
}

/// Opens `candidate` at `settings`: VBR at `video_kbps` (`maxrate` 1.5x, `bufsize` 2x), closed
/// GOP of `gop`, profile High (H.264) or Main (HEVC), BT.709 limited-range colour with left
/// chroma siting, global headers (MP4), `time_base = 1 / fps`.
pub fn open_encoder(
    candidate: EncoderCandidate,
    settings: &EncoderSettings,
) -> Result<OpenEncoder, EncoderError> {
    let (codec, encoder) = find(candidate)?;
    let mut context = AVCodecContext::new(&codec);
    let rate = settings.frame_rate;
    let bit_rate = i64::from(settings.video_kbps) * 1000;
    let pixel_format = candidate.kind.pixel_format();
    context.set_width(i32::try_from(settings.width).unwrap_or(i32::MAX));
    context.set_height(i32::try_from(settings.height).unwrap_or(i32::MAX));
    context.set_pix_fmt(pixel_format);
    context.set_time_base(ffi::AVRational {
        num: i32::try_from(rate.den()).unwrap_or(1),
        den: i32::try_from(rate.num()).unwrap_or(1),
    });
    context.set_framerate(ffi::AVRational {
        num: i32::try_from(rate.num()).unwrap_or(1),
        den: i32::try_from(rate.den()).unwrap_or(1),
    });
    context.set_sample_aspect_ratio(ffi::AVRational { num: 1, den: 1 });
    context.set_bit_rate(bit_rate);
    context.set_gop_size(i32::try_from(settings.gop).unwrap_or(i32::MAX));
    context.set_max_b_frames(b_frames(candidate));
    context.set_flags(
        context.flags
            | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32
            | ffi::AV_CODEC_FLAG_CLOSED_GOP as i32,
    );
    // SAFETY: plain fields of a codec context that is not open yet.
    unsafe {
        let raw = context.as_mut_ptr();
        (*raw).rc_max_rate = bit_rate * 3 / 2;
        (*raw).rc_buffer_size = i32::try_from(bit_rate * 2).unwrap_or(i32::MAX);
        (*raw).profile = match candidate.codec {
            VideoCodec::H264 => ffi::AV_PROFILE_H264_HIGH as i32,
            VideoCodec::Hevc => ffi::AV_PROFILE_HEVC_MAIN as i32,
        };
        if candidate.kind == EncoderKind::OpenH264 {
            let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
            (*raw).thread_count = i32::try_from((cores / 2).max(1)).unwrap_or(1);
        }
    }
    // The colour tags of export.md §4.6, in one place with the muxer's.
    super::mux::apply_color_tags(&mut context);
    set_private_options(&mut context, candidate, encoder)?;
    context.open(None).map_err(|err| EncoderError::Failed {
        encoder,
        stage: "open",
        detail: err.to_string(),
    })?;
    Ok(OpenEncoder {
        context,
        pixel_format,
        candidate,
    })
}

/// How long a test encode may wait on an encoder that keeps saying "try again".
const PROBE_STALL: Duration = Duration::from_secs(5);

/// Frames per test encode.
const PROBE_FRAMES: i64 = 8;

/// Test-encodes `kind` for `codec` with `Full`, then `Safe` if that differs: open at
/// [`EncoderSettings::PROBE`], send 8 grey frames, flush, and expect at least one packet.
/// Returns the option set that worked, or the error of the `Full` attempt.
///
/// Media Foundation needs COM on the calling thread: see [`ComGuard`].
pub fn probe(kind: EncoderKind, codec: VideoCodec) -> Result<OptionSet, EncoderError> {
    let attempt = |options| {
        test_encode(EncoderCandidate {
            kind,
            codec,
            options,
        })
    };
    match attempt(OptionSet::Full) {
        Ok(()) => Ok(OptionSet::Full),
        Err(full) if has_safe_variant(kind, codec) => match attempt(OptionSet::Safe) {
            Ok(()) => Ok(OptionSet::Safe),
            Err(safe) => {
                tracing::debug!(?kind, ?codec, %safe, "safe options failed too");
                Err(full)
            }
        },
        Err(full) => Err(full),
    }
}

fn test_encode(candidate: EncoderCandidate) -> Result<(), EncoderError> {
    let mut encoder = open_encoder(candidate, &EncoderSettings::PROBE)?;
    let name = find(candidate)?.1;
    let failed = |stage: &'static str, detail: String| EncoderError::Failed {
        encoder: name,
        stage,
        detail,
    };
    let mut frame = grey_frame(&EncoderSettings::PROBE, encoder.pixel_format)
        .map_err(|err| failed("frame", err.to_string()))?;
    let mut packets = 0usize;
    for pts in 0..PROBE_FRAMES {
        frame.set_pts(pts);
        send(&mut encoder.context, Some(&frame), &mut packets)
            .map_err(|err| failed("encode", err.to_string()))?;
    }
    send(&mut encoder.context, None, &mut packets)
        .map_err(|err| failed("flush", err.to_string()))?;
    let deadline = Instant::now() + PROBE_STALL;
    while !drain(&mut encoder.context, &mut packets)
        .map_err(|err| failed("flush", err.to_string()))?
    {
        if Instant::now() > deadline {
            return Err(failed("flush", "the encoder did not finish".into()));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if packets == 0 {
        return Err(failed("encode", "no packet".into()));
    }
    Ok(())
}

/// Sends `frame` (or the flush signal), draining packets while the encoder is full.
fn send(
    context: &mut AVCodecContext,
    frame: Option<&AVFrame>,
    packets: &mut usize,
) -> Result<(), RsmpegError> {
    let deadline = Instant::now() + PROBE_STALL;
    loop {
        match context.send_frame(frame) {
            Ok(()) => break,
            Err(RsmpegError::SendFrameAgainError) if Instant::now() < deadline => {
                if drain(context, packets)? {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(err) => return Err(err),
        }
    }
    drain(context, packets).map(|_| ())
}

/// Receives every packet that is ready. `true` once the encoder is fully flushed.
fn drain(context: &mut AVCodecContext, packets: &mut usize) -> Result<bool, RsmpegError> {
    loop {
        match context.receive_packet() {
            Ok(_) => *packets += 1,
            Err(RsmpegError::EncoderDrainError) => return Ok(false),
            Err(RsmpegError::EncoderFlushedError) => return Ok(true),
            Err(err) => return Err(err),
        }
    }
}

/// A frame of 50 % grey (Y' 126, Cb Cr 128) in `format` (NV12 or YUV 4:2:0 planar).
fn grey_frame(
    settings: &EncoderSettings,
    format: ffi::AVPixelFormat,
) -> Result<AVFrame, RsmpegError> {
    let mut frame = AVFrame::new();
    frame.set_width(i32::try_from(settings.width).unwrap_or(i32::MAX));
    frame.set_height(i32::try_from(settings.height).unwrap_or(i32::MAX));
    frame.set_format(format);
    frame.alloc_buffer()?;
    let height = settings.height as usize;
    let planes: &[(usize, usize, u8)] = if format == ffi::AV_PIX_FMT_NV12 {
        &[(0, height, 126), (1, height / 2, 128)]
    } else {
        &[(0, height, 126), (1, height / 2, 128), (2, height / 2, 128)]
    };
    for &(plane, rows, value) in planes {
        let linesize = usize::try_from(frame.linesize[plane]).unwrap_or(0);
        // SAFETY: `alloc_buffer` allocated `rows` rows of `linesize` bytes for this plane.
        unsafe { std::ptr::write_bytes(frame.data[plane], value, linesize * rows) };
    }
    Ok(frame)
}

/// `COM` initialised (multithreaded apartment) on the current thread for as long as this lives.
/// Media Foundation encoders need it; FFmpeg's own `MFStartup` is unaffected. Not `Send`: it must
/// be dropped on the thread that created it.
#[derive(Debug)]
pub struct ComGuard {
    initialized: bool,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ComGuard {
    pub fn multithreaded() -> Self {
        #[cfg(windows)]
        let initialized = {
            use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
            // SAFETY: called with no reserved pointer; paired with `CoUninitialize` in `drop` on
            // success (S_OK or S_FALSE), on the same thread (the guard is not `Send`).
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result.is_err() {
                tracing::warn!(%result, "CoInitializeEx failed; Media Foundation encoders may not work");
            }
            result.is_ok()
        };
        #[cfg(not(windows))]
        let initialized = false;
        Self {
            initialized,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.initialized {
            // SAFETY: balances the successful `CoInitializeEx` of `multithreaded` on this thread.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// `CUTFRAME_ENCODERS=<list>`: limits the candidates, e.g. `openH264` or `mfSoftware,openH264`
/// (CI and debugging, like `CUTFRAME_HWDECODE`).
pub const ENCODERS_ENV: &str = "CUTFRAME_ENCODERS";

/// The candidate limit from [`ENCODERS_ENV`]. The default allows every kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncoderFilter {
    only: Option<Vec<EncoderKind>>,
}

impl EncoderFilter {
    /// Only these kinds (in table order, whatever the order here).
    pub fn only(kinds: &[EncoderKind]) -> Self {
        Self {
            only: Some(kinds.to_vec()),
        }
    }

    /// Reads [`ENCODERS_ENV`]; unknown names are logged and ignored, and a value with no known
    /// name limits nothing.
    pub fn from_env() -> Self {
        let Ok(value) = std::env::var(ENCODERS_ENV) else {
            return Self::default();
        };
        let (filter, unknown) = Self::parse(&value);
        if !unknown.is_empty() {
            tracing::warn!(?unknown, "{ENCODERS_ENV}: unknown encoder names ignored");
        }
        if let Some(only) = &filter.only {
            tracing::info!(?only, "{ENCODERS_ENV} limits the export encoders");
        }
        filter
    }

    /// Parses a comma-separated list of keys (any case, spaces ignored). Returns the filter and
    /// the unknown names.
    pub fn parse(value: &str) -> (Self, Vec<String>) {
        let mut kinds = Vec::new();
        let mut unknown = Vec::new();
        for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match EncoderKind::ALL
                .into_iter()
                .find(|k| k.key().eq_ignore_ascii_case(name))
            {
                Some(kind) if !kinds.contains(&kind) => kinds.push(kind),
                Some(_) => {}
                None => unknown.push(name.to_owned()),
            }
        }
        let only = (!kinds.is_empty()).then_some(kinds);
        (Self { only }, unknown)
    }

    pub fn allows(&self, kind: EncoderKind) -> bool {
        self.only.as_ref().is_none_or(|only| only.contains(&kind))
    }
}

/// What the probe found out about one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Works with this option set.
    Available(OptionSet),
    /// The test encode failed (English detail, for the log).
    Failed(String),
    /// Left out by `CUTFRAME_ENCODERS`; not tried.
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeEntry {
    pub codec: VideoCodec,
    pub kind: EncoderKind,
    pub outcome: ProbeOutcome,
}

/// The session's probe result: every candidate of every codec, H.264 first, in table order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub entries: Vec<ProbeEntry>,
}

impl ProbeReport {
    /// Test-encodes every allowed candidate (COM initialised on this thread for the duration).
    /// Takes about 1-3 s; run it on a worker thread.
    pub fn run(filter: &EncoderFilter) -> Self {
        let _com = ComGuard::multithreaded();
        Self::run_with(filter, |kind, codec| {
            let started = Instant::now();
            let result = probe(kind, codec);
            let ms = started.elapsed().as_millis();
            match &result {
                Ok(options) => {
                    tracing::info!(?codec, ?kind, ?options, ms, "export encoder available")
                }
                Err(err) => tracing::info!(?codec, ?kind, %err, ms, "export encoder unavailable"),
            }
            result
        })
    }

    /// [`Self::run`] with another probe function (tests).
    pub fn run_with(
        filter: &EncoderFilter,
        mut probe: impl FnMut(EncoderKind, VideoCodec) -> Result<OptionSet, EncoderError>,
    ) -> Self {
        let mut entries = Vec::new();
        for codec in VideoCodec::ALL {
            for &kind in candidates(codec) {
                let outcome = if !filter.allows(kind) {
                    ProbeOutcome::Excluded
                } else {
                    match probe(kind, codec) {
                        Ok(options) => ProbeOutcome::Available(options),
                        Err(err) => ProbeOutcome::Failed(err.to_string()),
                    }
                };
                entries.push(ProbeEntry {
                    codec,
                    kind,
                    outcome,
                });
            }
        }
        Self { entries }
    }

    /// The candidates an export of `codec` tries, in order: the available ones of the table, or
    /// only `explicit` (empty if it is not available).
    pub fn candidates(
        &self,
        codec: VideoCodec,
        explicit: Option<EncoderKind>,
    ) -> Vec<EncoderCandidate> {
        self.entries
            .iter()
            .filter(|e| e.codec == codec && explicit.is_none_or(|k| k == e.kind))
            .filter_map(|e| match e.outcome {
                ProbeOutcome::Available(options) => Some(EncoderCandidate {
                    kind: e.kind,
                    codec,
                    options,
                }),
                _ => None,
            })
            .collect()
    }

    pub fn is_available(&self, kind: EncoderKind, codec: VideoCodec) -> bool {
        !self.candidates(codec, Some(kind)).is_empty()
    }
}

/// Encoders and muxers export and its follow-ups (task 0400) need from the linked FFmpeg.
pub const REQUIRED_ENCODERS: [&CStr; 14] = [
    c"h264_nvenc",
    c"hevc_nvenc",
    c"h264_qsv",
    c"hevc_qsv",
    c"h264_amf",
    c"hevc_amf",
    c"h264_mf",
    c"hevc_mf",
    c"libopenh264",
    c"aac",
    c"libmp3lame",
    c"pcm_s16le",
    c"png",
    c"mjpeg",
];
pub const REQUIRED_MUXERS: [&CStr; 4] = [c"mp4", c"ipod", c"mp3", c"wav"];

/// Checks the linked FFmpeg for every encoder and muxer in [`REQUIRED_ENCODERS`] and
/// [`REQUIRED_MUXERS`] (a guard for FFmpeg pin updates; logged at startup). Returns the missing
/// names.
pub fn required_encoders_present() -> Result<(), Vec<String>> {
    let mut missing: Vec<String> = REQUIRED_ENCODERS
        .iter()
        .filter(|name| AVCodec::find_encoder_by_name(name).is_none())
        .map(|name| name_str(name).to_owned())
        .collect();
    for muxer in REQUIRED_MUXERS {
        // SAFETY: `av_guess_format` only reads the NUL-terminated name; null filename and MIME
        // type are allowed.
        let format =
            unsafe { ffi::av_guess_format(muxer.as_ptr(), std::ptr::null(), std::ptr::null()) };
        if format.is_null() {
            missing.push(format!("{} (muxer)", name_str(muxer)));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests;
