//! The export plan (export.md §4.1): every number an export derives from the project and the
//! dialog's settings, computed once at start and shown in the dialog. Pure: no GPU, no FFmpeg,
//! no I/O.
//!
//! All arithmetic is integer (flicks, `u128`/`i128` intermediates), so the dialog and the export
//! loop always agree on sizes, frame and sample counts and bitrates.

use app_core::{FLICKS_PER_SECOND, FrameRate, Project, Time};

use super::encoders::EncoderKind;

/// The video codec of an export. H.264 is the default; HEVC needs a hardware encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 2] = [Self::H264, Self::Hevc];
}

/// Output size: the canvas, or a short side of 480 ... 2160 px with the canvas aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportResolution {
    Project,
    P480,
    P720,
    P1080,
    P1440,
    P2160,
}

impl ExportResolution {
    /// The target short side in pixels; `None` for the project size.
    pub const fn short_side(self) -> Option<u32> {
        match self {
            Self::Project => None,
            Self::P480 => Some(480),
            Self::P720 => Some(720),
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
            Self::P2160 => Some(2160),
        }
    }
}

/// Quality as a factor on YouTube's upload bitrate, or an explicit bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportQuality {
    Low,
    Recommended,
    High,
    /// A bitrate in [`CUSTOM_KBPS`], taken as given.
    Custom {
        kbps: u32,
    },
}

/// What the dialog chose for a video export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoExportSettings {
    pub codec: VideoCodec,
    pub resolution: ExportResolution,
    /// `None`: the project rate, or 60 fps if the project rate is above 60.
    pub frame_rate: Option<FrameRate>,
    pub quality: ExportQuality,
    /// `None`: automatic (export.md §4.4). `Some`: only this encoder, no fallback.
    pub encoder: Option<EncoderKind>,
}

/// Everything the export loop needs, derived once at start and shown in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportPlan {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Frames `0..frames`; frame `n` shows the timeline at `n x frame_duration`.
    pub frames: u32,
    pub sample_rate: u32,
    /// Audio samples per channel: audio ends with the last video frame (within one sample).
    pub samples: u64,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    /// Keyframe interval in frames: half the frame rate, at least 1.
    pub gop: u32,
    pub estimated_bytes: u64,
}

impl ExportPlan {
    /// Length of the exported video, `frames x frame_duration`: at or just past the timeline end.
    pub fn duration(&self) -> Time {
        Time::from_flicks(i64::from(self.frames) * self.frame_rate.frame_duration().flicks())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("the timeline is empty")]
    EmptyTimeline,
    #[error("frame rate {num}/{den} is not allowed for export")]
    InvalidFrameRate { num: u32, den: u32 },
    #[error("custom bitrate {kbps} kbps is outside 500..=200000")]
    InvalidBitrate { kbps: u32 },
    #[error("the timeline has more frames or samples than an export supports")]
    TooLong,
}

/// AAC-LC stereo at YouTube's recommended audio bitrate.
pub const AUDIO_KBPS: u32 = 384;

/// Accepted range of [`ExportQuality::Custom`]; derived bitrates are clamped to it as well.
pub const CUSTOM_KBPS: std::ops::RangeInclusive<u32> = 500..=200_000;

/// The fixed export frame rates as `(num, den)`; the project rate is allowed too if it is at most
/// 60 fps.
pub const EXPORT_FRAME_RATES: [(u32, u32); 5] = [(24, 1), (25, 1), (30, 1), (50, 1), (60, 1)];

/// Smallest exported side, in pixels.
const MIN_SIDE: u32 = 16;

/// Derived bitrates are rounded to a multiple of this, in kbps.
const KBPS_STEP: u128 = 500;

/// YouTube's SDR upload bitrates in kbps by class: (short side, width of its 16:9 reference
/// frame, <= 30 fps, > 30 fps). Largest first.
const BITRATE_TABLE: [(u32, u32, u32, u32); 5] = [
    (2160, 3840, 45_000, 68_000),
    (1440, 2560, 16_000, 24_000),
    (1080, 1920, 8_000, 12_000),
    (720, 1280, 5_000, 7_500),
    (480, 854, 2_500, 4_000),
];

/// Derives the plan for exporting `project` with `settings`.
pub fn plan_video_export(
    project: &Project,
    settings: &VideoExportSettings,
) -> Result<ExportPlan, PlanError> {
    let canvas = &project.settings;
    let duration = project.duration();
    if duration <= Time::ZERO {
        return Err(PlanError::EmptyTimeline);
    }
    let frame_rate = export_frame_rate(canvas.frame_rate, settings.frame_rate)?;
    let (width, height) = output_size(canvas.width, canvas.height, settings.resolution);

    let frame_flicks = frame_rate.frame_duration().flicks();
    // ceil(D / F); D > 0
    let frames = u32::try_from((duration.flicks() - 1) / frame_flicks + 1)
        .map_err(|_| PlanError::TooLong)?;
    let video_flicks = i128::from(frames) * i128::from(frame_flicks);
    let samples = u64::try_from(
        video_flicks * i128::from(canvas.sample_rate) / i128::from(FLICKS_PER_SECOND),
    )
    .map_err(|_| PlanError::TooLong)?;

    let video_kbps = match settings.quality {
        ExportQuality::Custom { kbps } if !CUSTOM_KBPS.contains(&kbps) => {
            return Err(PlanError::InvalidBitrate { kbps });
        }
        quality => video_kbps(width, height, frame_rate, quality, settings.codec),
    };
    Ok(ExportPlan {
        width,
        height,
        frame_rate,
        frames,
        sample_rate: canvas.sample_rate,
        samples,
        video_kbps,
        audio_kbps: AUDIO_KBPS,
        gop: gop(frame_rate),
        estimated_bytes: estimated_bytes(video_kbps + AUDIO_KBPS, video_flicks),
    })
}

/// `requested` if export allows it; `None` picks the project rate, or 60 fps above 60.
pub fn export_frame_rate(
    project: FrameRate,
    requested: Option<FrameRate>,
) -> Result<FrameRate, PlanError> {
    let Some(rate) = requested else {
        return Ok(if at_most(project, FrameRate::FPS_60) {
            project
        } else {
            FrameRate::FPS_60
        });
    };
    let fixed = EXPORT_FRAME_RATES
        .iter()
        .any(|&(num, den)| same_rate(rate, num, den));
    let project_rate =
        same_rate(rate, project.num(), project.den()) && at_most(rate, FrameRate::FPS_60);
    if fixed || project_rate {
        Ok(rate)
    } else {
        Err(PlanError::InvalidFrameRate {
            num: rate.num(),
            den: rate.den(),
        })
    }
}

/// The canvas size, or both sides scaled so the short side is the target, each rounded to the
/// nearest even number (halves up), at least 16 px. Upscaling is allowed.
pub fn output_size(canvas_w: u32, canvas_h: u32, resolution: ExportResolution) -> (u32, u32) {
    let Some(target) = resolution.short_side() else {
        return (canvas_w, canvas_h);
    };
    let short = u64::from(canvas_w.min(canvas_h).max(1));
    let scale = |side: u32| {
        // round(side x target / short / 2) x 2, halves up
        let even = (u64::from(side) * u64::from(target) + short) / (2 * short) * 2;
        u32::try_from(even).unwrap_or(u32::MAX).max(MIN_SIDE)
    };
    (scale(canvas_w), scale(canvas_h))
}

/// YouTube's bitrate for the output's class, scaled by area, quality and codec, rounded to the
/// nearest 500 kbps (halves up) and clamped to [`CUSTOM_KBPS`]. A `Custom` quality is returned
/// as given.
pub fn video_kbps(
    width: u32,
    height: u32,
    rate: FrameRate,
    quality: ExportQuality,
    codec: VideoCodec,
) -> u32 {
    // Factors in tenths: quality Low 1.0, Recommended 1.5, High 2.5; codec H.264 1.0, HEVC 0.6.
    let quality_tenths: u128 = match quality {
        ExportQuality::Low => 10,
        ExportQuality::Recommended => 15,
        ExportQuality::High => 25,
        ExportQuality::Custom { kbps } => return kbps,
    };
    let codec_tenths: u128 = match codec {
        VideoCodec::H264 => 10,
        VideoCodec::Hevc => 6,
    };
    let short = width.min(height);
    let (class, class_w, low_rate, high_rate) = BITRATE_TABLE
        .into_iter()
        .find(|&(class, ..)| class <= short)
        .unwrap_or(BITRATE_TABLE[BITRATE_TABLE.len() - 1]);
    let base = if at_most(rate, FrameRate::FPS_30) {
        low_rate
    } else {
        high_rate
    };
    let class_area = u128::from(class_w) * u128::from(class);
    let numerator =
        u128::from(base) * quality_tenths * codec_tenths * u128::from(width) * u128::from(height);
    let denominator = 100 * class_area;
    // round(numerator / denominator / STEP), halves up
    let steps = (2 * numerator + KBPS_STEP * denominator) / (2 * KBPS_STEP * denominator);
    let kbps = u32::try_from(steps * KBPS_STEP).unwrap_or(u32::MAX);
    kbps.clamp(*CUSTOM_KBPS.start(), *CUSTOM_KBPS.end())
}

/// Keyframe interval: `round(fps / 2)` (halves up), at least 1.
pub fn gop(rate: FrameRate) -> u32 {
    let (num, den) = (u64::from(rate.num()), u64::from(rate.den()));
    u32::try_from((num + den) / (2 * den))
        .unwrap_or(u32::MAX)
        .max(1)
}

/// `kbps x 125 bytes/s x duration x 1.05` (container overhead and VBR overshoot).
fn estimated_bytes(total_kbps: u32, video_flicks: i128) -> u64 {
    let bytes =
        i128::from(total_kbps) * 125 * video_flicks * 105 / (100 * i128::from(FLICKS_PER_SECOND));
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// `a <= b`, exactly.
fn at_most(a: FrameRate, b: FrameRate) -> bool {
    u64::from(a.num()) * u64::from(b.den()) <= u64::from(b.num()) * u64::from(a.den())
}

/// `rate == num / den`, exactly (unreduced fractions included).
fn same_rate(rate: FrameRate, num: u32, den: u32) -> bool {
    u64::from(rate.num()) * u64::from(den) == u64::from(num) * u64::from(rate.den())
}

#[cfg(test)]
mod tests;
