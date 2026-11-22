//! Media assets: imported video, audio and image files and what the probe found out about them
//! (design `docs/design/media-import.md` §3).
//!
//! Plain data like the rest of the model. Probing, fingerprinting and decoding live in
//! `engine::media`; this module only describes the result and checks its invariants.

use serde::{Deserialize, Serialize};

use crate::model::id_type;
use crate::time::{Rational, Time};

id_type!(
    /// Identifies a media asset within a project.
    MediaId
);

impl std::fmt::Display for MediaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: MediaId,
    pub kind: MediaKind,
    pub path: MediaPath,
    pub fingerprint: Fingerprint,
    pub info: MediaInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    /// Has a video stream with a duration (includes animated GIF). May have audio.
    Video,
    /// Audio streams only.
    Audio,
    /// A still picture: a video stream with one frame and no duration.
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPath {
    /// Absolute path, UTF-8, without the `\\?\` prefix.
    pub absolute: String,
    /// Relative to the project file's directory. Written on save (task 0100); `None` before the
    /// project has a location or when the media is on another drive.
    pub relative: Option<String>,
}

impl MediaPath {
    /// A path known only by its absolute form (every import until task 0100 lands).
    pub fn absolute(path: impl Into<String>) -> Self {
        Self {
            absolute: path.into(),
            relative: None,
        }
    }
}

/// Content identity: the cache key and the relink match. Never sent over IPC.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    /// File size in bytes.
    pub size: u64,
    /// Last-modified time, milliseconds since the Unix epoch. Only a hint for "unchanged" checks.
    pub modified: i64,
    /// 32 lowercase hex chars: BLAKE3 over `size` (LE u64) + the first 1 MiB + the last 1 MiB,
    /// truncated to 128 bits.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// FFmpeg demuxer short name (the first of its names), e.g. `mov`, `matroska`, `mp3`.
    pub container: String,
    /// Container time, in flicks, of media time zero (the earliest start of the used streams).
    /// Media time `t` is container time `t + origin` for every stream.
    pub origin: Time,
    /// `None` only for [`MediaKind::Image`].
    pub duration: Option<Time>,
    pub video: Option<VideoStreamInfo>,
    /// Every decodable audio stream, default stream first. Attached pictures and data streams are
    /// not listed.
    pub audio: Vec<AudioStreamInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStreamInfo {
    pub stream_index: u32,
    /// FFmpeg codec name, e.g. `h264`, `hevc`, `vp9`, `prores`.
    pub codec: String,
    /// Decoded picture size before rotation and pixel-aspect correction.
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio; 1/1 when the file has none.
    pub sample_aspect: Rational,
    /// Clockwise quarter turns that make the picture upright (display matrix or EXIF).
    pub rotation: Rotation,
    /// Average frame rate as reported by the demuxer. Informational (media panel, a suggested
    /// project rate); never used to locate frames. `None` for images.
    pub frame_rate: Option<Rational>,
    /// FFmpeg pixel format name, e.g. `yuv420p10le`.
    pub pixel_format: String,
    /// Bits per luma sample from the pixel format descriptor (8, 10, 12).
    pub bit_depth: u8,
    pub color: ColorInfo,
    /// `Some` when the stream needs the HDR tone map; see [`hdr_transfer`] for the rule.
    pub hdr: Option<HdrInfo>,
}

impl VideoStreamInfo {
    /// The upright picture size on screen: the width corrected by `sample_aspect` (rounded to an
    /// even number), then width and height swapped for quarter turns. Thumbnails and the render
    /// graph both use it, so the media panel and the preview agree on the shape.
    pub fn display_size(&self) -> (u32, u32) {
        let width = scale_to_even(self.width, self.sample_aspect);
        if self.rotation.swaps_axes() {
            (self.height, width)
        } else {
            (width, self.height)
        }
    }
}

/// `value * ratio` rounded to the nearest even number, at least 2.
fn scale_to_even(value: u32, ratio: Rational) -> u32 {
    let scaled = u64::from(value) * u64::from(ratio.num());
    let den = u64::from(ratio.den());
    let even = (scaled + den) / (2 * den) * 2;
    u32::try_from(even).unwrap_or(!1).max(2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HdrInfo {
    /// Which curve the decoded base layer uses. This is what the tone map keys on.
    pub transfer: HdrTransfer,
    /// Peak luminance of the mastering display in whole cd/m² (nits), when present.
    pub mastering_max_nits: Option<u32>,
    /// MaxCLL in nits, when present.
    pub max_cll_nits: Option<u32>,
    /// The Dolby Vision configuration record, when present.
    pub dolby_vision: Option<DolbyVisionInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum HdrTransfer {
    Pq,
    Hlg,
    /// Dolby Vision without a compatible base layer (profile 5): the simple tone map shows wrong
    /// colours. Imported, logged and badged; no Dolby Vision processing.
    DolbyVisionOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DolbyVisionInfo {
    /// E.g. 8 for iPhone footage.
    pub profile: u8,
    pub level: u8,
    /// `bl_signal_compatibility_id`: 1 = HDR10 (PQ), 2 = SDR, 4 = HLG (iPhone 8.4), 6 = HDR10
    /// (PQ), 0 = none.
    pub compatibility_id: u8,
}

/// FFmpeg's name of the PQ transfer function.
pub const TRANSFER_PQ: &str = "smpte2084";
/// FFmpeg's name of the HLG transfer function.
pub const TRANSFER_HLG: &str = "arib-std-b67";

/// The HDR curve of a video stream with colour transfer `transfer` (FFmpeg's name) and an
/// optional Dolby Vision record, or `None` for SDR. The one rule for the probe and for validation:
///
/// - Dolby Vision without a compatible base layer (`compatibility_id` 0, profile 5) is
///   [`HdrTransfer::DolbyVisionOnly`], whatever the stream's tags say;
/// - otherwise a PQ or HLG transfer tag decides;
/// - otherwise the Dolby Vision base layer decides: 1 (HDR10) and 6 (HDR10, as in profile 8.6 / Blu-ray
///   style streams) are PQ, 4 (HLG) is HLG. A record
///   with an SDR base layer (2) is SDR: the base layer plays without a tone map.
pub fn hdr_transfer(
    transfer: Option<&str>,
    dolby_vision: Option<DolbyVisionInfo>,
) -> Option<HdrTransfer> {
    if dolby_vision.is_some_and(|dv| dv.compatibility_id == 0) {
        return Some(HdrTransfer::DolbyVisionOnly);
    }
    match transfer {
        Some(TRANSFER_PQ) => return Some(HdrTransfer::Pq),
        Some(TRANSFER_HLG) => return Some(HdrTransfer::Hlg),
        _ => {}
    }
    match dolby_vision.map(|dv| dv.compatibility_id) {
        Some(1 | 6) => Some(HdrTransfer::Pq),
        Some(4) => Some(HdrTransfer::Hlg),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    pub stream_index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// FFmpeg's own names (`av_color_*_name`), `None` when unspecified. Strings, not enums, so new
/// values never need a migration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorInfo {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub matrix: Option<String>,
    pub range: Option<String>,
}

/// Clockwise quarter turns that make a picture upright.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    /// The rotation nearest to `degrees` clockwise, any value (e.g. -90 is `Cw270`).
    pub fn from_degrees_cw(degrees: f64) -> Self {
        if !degrees.is_finite() {
            return Self::None;
        }
        match ((degrees / 90.0).round() as i64).rem_euclid(4) {
            1 => Self::Cw90,
            2 => Self::Cw180,
            3 => Self::Cw270,
            _ => Self::None,
        }
    }

    /// True for 90° and 270°: the upright picture is the stored one with width and height swapped.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Self::Cw90 | Self::Cw270)
    }
}

impl MediaAsset {
    /// The file name, from the absolute path (either separator).
    pub fn file_name(&self) -> &str {
        let path = self.path.absolute.as_str();
        path.rsplit(['/', '\\']).next().unwrap_or(path)
    }
}

/// A broken media invariant (ADR-0002 §3, design §3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("media {id}: {problem}")]
pub struct MediaValidationError {
    pub id: MediaId,
    pub problem: MediaProblem,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaProblem {
    #[error("the id is used by another media asset")]
    DuplicateId,
    #[error("the absolute path is empty")]
    EmptyPath,
    #[error("the fingerprint hash is not 32 lowercase hex characters")]
    InvalidHash,
    #[error("{0:?} media must have a video stream")]
    MissingVideo(MediaKind),
    #[error("audio media must not have a video stream")]
    UnexpectedVideo,
    #[error("{0:?} media must have a positive duration")]
    MissingDuration(MediaKind),
    #[error("images have no duration")]
    UnexpectedDuration,
    #[error("video media must have a frame rate")]
    MissingFrameRate,
    #[error("audio media must have an audio stream")]
    MissingAudio,
    #[error("images have no audio streams")]
    UnexpectedAudio,
    #[error("the picture size is empty")]
    EmptySize,
    #[error("bit depth {0} is outside 8..=16")]
    InvalidBitDepth(u8),
    #[error("an audio stream has no sample rate or no channels")]
    InvalidAudioStream,
    #[error("the HDR info does not match the colour transfer and Dolby Vision record")]
    HdrMismatch,
}

/// Checks the media invariants of a project. `Project::validate` (task 0100) calls this; until
/// then [`crate::command::AddMedia`] checks every asset it adds.
pub fn validate_media(media: &[MediaAsset]) -> Result<(), MediaValidationError> {
    let mut seen = std::collections::HashSet::with_capacity(media.len());
    for asset in media {
        if !seen.insert(asset.id) {
            return Err(MediaValidationError {
                id: asset.id,
                problem: MediaProblem::DuplicateId,
            });
        }
        validate_asset(asset).map_err(|problem| MediaValidationError {
            id: asset.id,
            problem,
        })?;
    }
    Ok(())
}

/// The invariants of one asset on its own (everything but id uniqueness).
pub fn validate_asset(asset: &MediaAsset) -> Result<(), MediaProblem> {
    use MediaProblem as P;

    if asset.path.absolute.is_empty() {
        return Err(P::EmptyPath);
    }
    let hash = &asset.fingerprint.hash;
    if hash.len() != 32 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(P::InvalidHash);
    }

    let info = &asset.info;
    match asset.kind {
        MediaKind::Video | MediaKind::Image if info.video.is_none() => {
            return Err(P::MissingVideo(asset.kind));
        }
        MediaKind::Audio if info.video.is_some() => return Err(P::UnexpectedVideo),
        _ => {}
    }
    match (asset.kind, info.duration) {
        (MediaKind::Image, Some(_)) => return Err(P::UnexpectedDuration),
        (MediaKind::Video | MediaKind::Audio, d) if d.is_none_or(|d| d <= Time::ZERO) => {
            return Err(P::MissingDuration(asset.kind));
        }
        _ => {}
    }
    match asset.kind {
        MediaKind::Audio if info.audio.is_empty() => return Err(P::MissingAudio),
        MediaKind::Image if !info.audio.is_empty() => return Err(P::UnexpectedAudio),
        MediaKind::Video if info.video.as_ref().is_some_and(|v| v.frame_rate.is_none()) => {
            return Err(P::MissingFrameRate);
        }
        _ => {}
    }
    if info
        .audio
        .iter()
        .any(|a| a.sample_rate == 0 || a.channels == 0)
    {
        return Err(P::InvalidAudioStream);
    }

    if let Some(video) = &info.video {
        if video.width == 0 || video.height == 0 {
            return Err(P::EmptySize);
        }
        if !(8..=16).contains(&video.bit_depth) {
            return Err(P::InvalidBitDepth(video.bit_depth));
        }
        let transfer = video.color.transfer.as_deref();
        let consistent = match &video.hdr {
            Some(hdr) => hdr_transfer(transfer, hdr.dolby_vision) == Some(hdr.transfer),
            None => hdr_transfer(transfer, None).is_none(),
        };
        if !consistent {
            return Err(P::HdrMismatch);
        }
    }
    Ok(())
}

/// A valid asset of each kind, for tests in this crate and in the shell.
pub mod samples {
    use super::*;

    fn hash(seed: u8) -> String {
        format!("{seed:02x}").repeat(16)
    }

    fn video_stream(width: u32, height: u32) -> VideoStreamInfo {
        VideoStreamInfo {
            stream_index: 0,
            codec: "h264".into(),
            width,
            height,
            sample_aspect: Rational::ONE,
            rotation: Rotation::None,
            frame_rate: Some(Rational::new(30, 1).expect("valid")),
            pixel_format: "yuv420p".into(),
            bit_depth: 8,
            color: ColorInfo::default(),
            hdr: None,
        }
    }

    fn stereo() -> AudioStreamInfo {
        AudioStreamInfo {
            stream_index: 1,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
        }
    }

    /// A 5 s 1920x1080 H.264 + AAC clip. `seed` varies the path and the hash.
    pub fn video(seed: u8) -> MediaAsset {
        MediaAsset {
            id: MediaId::new(),
            kind: MediaKind::Video,
            path: MediaPath::absolute(format!("C:\\Media\\clip-{seed}.mp4")),
            fingerprint: Fingerprint {
                size: 1000 + u64::from(seed),
                modified: 1_763_800_000_000,
                hash: hash(seed),
            },
            info: MediaInfo {
                container: "mov".into(),
                origin: Time::ZERO,
                duration: Some(Time::from_seconds(5)),
                video: Some(video_stream(1920, 1080)),
                audio: vec![stereo()],
            },
        }
    }

    /// A 3 s stereo MP3.
    pub fn audio(seed: u8) -> MediaAsset {
        let mut asset = video(seed);
        asset.kind = MediaKind::Audio;
        asset.path = MediaPath::absolute(format!("C:\\Media\\song-{seed}.mp3"));
        asset.info = MediaInfo {
            container: "mp3".into(),
            origin: Time::ZERO,
            duration: Some(Time::from_seconds(3)),
            video: None,
            audio: vec![AudioStreamInfo {
                stream_index: 0,
                codec: "mp3".into(),
                ..stereo()
            }],
        };
        asset
    }

    /// A 4032x3024 photo.
    pub fn image(seed: u8) -> MediaAsset {
        let mut asset = video(seed);
        asset.kind = MediaKind::Image;
        asset.path = MediaPath::absolute(format!("C:\\Media\\photo-{seed}.jpg"));
        let mut stream = video_stream(4032, 3024);
        stream.codec = "mjpeg".into();
        stream.pixel_format = "yuvj420p".into();
        stream.frame_rate = None;
        asset.info = MediaInfo {
            container: "image2".into(),
            origin: Time::ZERO,
            duration: None,
            video: Some(stream),
            audio: Vec::new(),
        };
        asset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rational(num: i64, den: i64) -> Rational {
        Rational::new(num, den).unwrap()
    }

    fn problem(asset: &MediaAsset) -> Option<MediaProblem> {
        validate_asset(asset).err()
    }

    fn with_video(mut asset: MediaAsset, edit: impl FnOnce(&mut VideoStreamInfo)) -> MediaAsset {
        edit(asset.info.video.as_mut().expect("has video"));
        asset
    }

    #[test]
    fn samples_are_valid() {
        let media = [samples::video(1), samples::audio(2), samples::image(3)];
        assert_eq!(validate_media(&media), Ok(()));
    }

    #[test]
    fn display_size_applies_pixel_aspect_then_rotation() {
        let anamorphic = with_video(samples::video(1), |v| {
            v.width = 1440;
            v.sample_aspect = rational(4, 3);
        });
        assert_eq!(anamorphic.info.video.unwrap().display_size(), (1920, 1080));

        let portrait = with_video(samples::video(1), |v| v.rotation = Rotation::Cw90);
        assert_eq!(portrait.info.video.unwrap().display_size(), (1080, 1920));

        // 720x576 PAL 16:9 (SAR 64:45): 1024 wide.
        let pal = with_video(samples::video(1), |v| {
            (v.width, v.height, v.sample_aspect) = (720, 576, rational(64, 45));
            v.rotation = Rotation::Cw270;
        });
        assert_eq!(pal.info.video.unwrap().display_size(), (576, 1024));

        // Odd results are rounded to even.
        let odd = with_video(samples::video(1), |v| {
            (v.width, v.sample_aspect) = (101, rational(1, 1));
        });
        assert_eq!(odd.info.video.unwrap().display_size().0, 102);
    }

    #[test]
    fn rotation_snaps_to_quarter_turns() {
        assert_eq!(Rotation::from_degrees_cw(0.0), Rotation::None);
        assert_eq!(Rotation::from_degrees_cw(89.4), Rotation::Cw90);
        assert_eq!(Rotation::from_degrees_cw(-90.0), Rotation::Cw270);
        assert_eq!(Rotation::from_degrees_cw(180.0), Rotation::Cw180);
        assert_eq!(Rotation::from_degrees_cw(-180.0), Rotation::Cw180);
        assert_eq!(Rotation::from_degrees_cw(450.0), Rotation::Cw90);
        assert_eq!(Rotation::from_degrees_cw(f64::NAN), Rotation::None);
    }

    #[test]
    fn file_name_takes_either_separator() {
        let mut asset = samples::video(1);
        assert_eq!(asset.file_name(), "clip-1.mp4");
        asset.path.absolute = "/home/me/ąęłó/a.mov".into();
        assert_eq!(asset.file_name(), "a.mov");
    }

    #[test]
    fn hdr_rule() {
        let dv = |compatibility_id| {
            Some(DolbyVisionInfo {
                profile: 8,
                level: 4,
                compatibility_id,
            })
        };
        assert_eq!(hdr_transfer(None, None), None);
        assert_eq!(hdr_transfer(Some("bt709"), None), None);
        assert_eq!(hdr_transfer(Some(TRANSFER_PQ), None), Some(HdrTransfer::Pq));
        assert_eq!(
            hdr_transfer(Some(TRANSFER_HLG), None),
            Some(HdrTransfer::Hlg)
        );
        // iPhone: profile 8.4, HLG base layer.
        assert_eq!(
            hdr_transfer(Some(TRANSFER_HLG), dv(4)),
            Some(HdrTransfer::Hlg)
        );
        assert_eq!(hdr_transfer(None, dv(4)), Some(HdrTransfer::Hlg));
        assert_eq!(hdr_transfer(None, dv(1)), Some(HdrTransfer::Pq));
        // Compatibility id 6 is an HDR10 base layer too.
        assert_eq!(hdr_transfer(None, dv(6)), Some(HdrTransfer::Pq));
        assert_eq!(hdr_transfer(Some("bt709"), dv(2)), None);
        // Profile 5 never takes the PQ/HLG path, whatever the tags say.
        assert_eq!(
            hdr_transfer(Some(TRANSFER_PQ), dv(0)),
            Some(HdrTransfer::DolbyVisionOnly)
        );
    }

    #[test]
    fn validate_accepts_hdr_combinations() {
        let hlg_dv = with_video(samples::video(1), |v| {
            v.color.transfer = Some(TRANSFER_HLG.into());
            v.bit_depth = 10;
            v.hdr = Some(HdrInfo {
                transfer: HdrTransfer::Hlg,
                mastering_max_nits: None,
                max_cll_nits: None,
                dolby_vision: Some(DolbyVisionInfo {
                    profile: 8,
                    level: 4,
                    compatibility_id: 4,
                }),
            });
        });
        assert_eq!(problem(&hlg_dv), None);
        let pq = with_video(samples::video(1), |v| {
            v.color.transfer = Some(TRANSFER_PQ.into());
            v.hdr = Some(HdrInfo {
                transfer: HdrTransfer::Pq,
                mastering_max_nits: Some(1000),
                max_cll_nits: Some(400),
                dolby_vision: None,
            });
        });
        assert_eq!(problem(&pq), None);
        let profile5 = with_video(samples::video(1), |v| {
            v.hdr = Some(HdrInfo {
                transfer: HdrTransfer::DolbyVisionOnly,
                mastering_max_nits: None,
                max_cll_nits: None,
                dolby_vision: Some(DolbyVisionInfo {
                    profile: 5,
                    level: 6,
                    compatibility_id: 0,
                }),
            });
        });
        assert_eq!(problem(&profile5), None);
    }

    /// One case per invariant of design §3.
    #[test]
    fn validate_rejects_each_broken_invariant() {
        use MediaProblem as P;

        let video = samples::video(1);
        let duplicate = [video.clone(), video.clone()];
        assert_eq!(
            validate_media(&duplicate).unwrap_err().problem,
            P::DuplicateId
        );

        let mut a = samples::video(1);
        a.path.absolute.clear();
        assert_eq!(problem(&a), Some(P::EmptyPath));

        for bad in [
            "",
            "ABCDEFABCDEFABCDEFABCDEFABCDEFAB",
            "0123",
            &"g".repeat(32),
        ] {
            let mut a = samples::video(1);
            a.fingerprint.hash = bad.into();
            assert_eq!(problem(&a), Some(P::InvalidHash), "{bad}");
        }

        let mut a = samples::video(1);
        a.info.video = None;
        assert_eq!(problem(&a), Some(P::MissingVideo(MediaKind::Video)));
        let mut a = samples::image(1);
        a.info.video = None;
        assert_eq!(problem(&a), Some(P::MissingVideo(MediaKind::Image)));
        let mut a = samples::audio(1);
        a.info.video = samples::video(1).info.video;
        assert_eq!(problem(&a), Some(P::UnexpectedVideo));

        let mut a = samples::video(1);
        a.info.duration = None;
        assert_eq!(problem(&a), Some(P::MissingDuration(MediaKind::Video)));
        let mut a = samples::audio(1);
        a.info.duration = Some(Time::ZERO);
        assert_eq!(problem(&a), Some(P::MissingDuration(MediaKind::Audio)));
        let mut a = samples::image(1);
        a.info.duration = Some(Time::from_seconds(1));
        assert_eq!(problem(&a), Some(P::UnexpectedDuration));

        let a = with_video(samples::video(1), |v| v.frame_rate = None);
        assert_eq!(problem(&a), Some(P::MissingFrameRate));
        let mut a = samples::audio(1);
        a.info.audio.clear();
        assert_eq!(problem(&a), Some(P::MissingAudio));
        let mut a = samples::image(1);
        a.info.audio = samples::audio(1).info.audio;
        assert_eq!(problem(&a), Some(P::UnexpectedAudio));
        let mut a = samples::audio(1);
        a.info.audio[0].channels = 0;
        assert_eq!(problem(&a), Some(P::InvalidAudioStream));

        let a = with_video(samples::video(1), |v| v.height = 0);
        assert_eq!(problem(&a), Some(P::EmptySize));
        let a = with_video(samples::video(1), |v| v.bit_depth = 7);
        assert_eq!(problem(&a), Some(P::InvalidBitDepth(7)));

        // PQ transfer without HDR info, HDR info on SDR, and the wrong curve.
        let a = with_video(samples::video(1), |v| {
            v.color.transfer = Some(TRANSFER_PQ.into());
        });
        assert_eq!(problem(&a), Some(P::HdrMismatch));
        let hdr = HdrInfo {
            transfer: HdrTransfer::Hlg,
            mastering_max_nits: None,
            max_cll_nits: None,
            dolby_vision: None,
        };
        let a = with_video(samples::video(1), |v| v.hdr = Some(hdr));
        assert_eq!(problem(&a), Some(P::HdrMismatch));
        let a = with_video(samples::video(1), |v| {
            v.color.transfer = Some(TRANSFER_PQ.into());
            v.hdr = Some(hdr);
        });
        assert_eq!(problem(&a), Some(P::HdrMismatch));
        // DolbyVisionOnly needs compatibility id 0.
        let a = with_video(samples::video(1), |v| {
            v.hdr = Some(HdrInfo {
                transfer: HdrTransfer::DolbyVisionOnly,
                dolby_vision: Some(DolbyVisionInfo {
                    profile: 8,
                    level: 4,
                    compatibility_id: 4,
                }),
                ..hdr
            });
        });
        assert_eq!(problem(&a), Some(P::HdrMismatch));
    }
}
