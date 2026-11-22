//! Opening a media file and reading its stream info.
//!
//! This does file I/O and may take a while on slow disks or large files: run it in a worker job,
//! never inside an IPC handler (ADR-0004).

use std::ffi::CString;
use std::path::{Path, PathBuf};

use app_core::{FLICKS_PER_SECOND, Time};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

/// Why a file could not be probed.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// FFmpeg takes UTF-8 paths (it converts them to UTF-16 on Windows itself).
    #[error("path is not valid UTF-8 or contains a NUL byte: {0}")]
    InvalidPath(PathBuf),
    #[error("cannot open {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: RsmpegError,
    },
}

/// What a stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video { width: u32, height: u32 },
    Audio { sample_rate: u32, channels: u32 },
    Other,
}

/// One stream of a media file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub index: usize,
    /// FFmpeg codec name, e.g. `h264`, `aac`.
    pub codec: String,
    pub kind: StreamKind,
    /// Stream duration, if the container states it.
    pub duration: Option<Time>,
}

/// Container-level info of a media file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    /// FFmpeg demuxer name(s), e.g. `mov,mp4,m4a,3gp,3g2,mj2`.
    pub format: String,
    /// Container duration, if known.
    pub duration: Option<Time>,
    pub streams: Vec<StreamInfo>,
}

/// Converts `ts` in units of `num/den` seconds to flicks, rounded to the nearest flick.
/// `None` for `AV_NOPTS_VALUE`, an invalid time base or a result outside `i64`.
pub(crate) fn timestamp_to_time(ts: i64, num: i32, den: i32) -> Option<Time> {
    if ts == ffi::AV_NOPTS_VALUE || num <= 0 || den <= 0 {
        return None;
    }
    let numerator = i128::from(ts) * i128::from(num) * i128::from(FLICKS_PER_SECOND);
    let den = i128::from(den);
    // Round half away from zero.
    let half = if numerator >= 0 { den / 2 } else { -(den / 2) };
    let flicks = (numerator + half) / den;
    i64::try_from(flicks).ok().map(Time::from_flicks)
}

/// Opens `path` and reads its container and stream info. Does not decode any frames.
pub fn probe(path: &Path) -> Result<MediaInfo, ProbeError> {
    let url = path
        .to_str()
        .and_then(|s| CString::new(s).ok())
        .ok_or_else(|| ProbeError::InvalidPath(path.to_owned()))?;
    let input = AVFormatContextInput::open(&url).map_err(|source| ProbeError::Open {
        path: path.to_owned(),
        source,
    })?;

    let streams = input
        .streams()
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let par = stream.codecpar();
            let media_type = par.codec_type();
            let kind = if media_type.is_video() {
                StreamKind::Video {
                    width: u32::try_from(par.width).unwrap_or(0),
                    height: u32::try_from(par.height).unwrap_or(0),
                }
            } else if media_type.is_audio() {
                StreamKind::Audio {
                    sample_rate: u32::try_from(par.sample_rate).unwrap_or(0),
                    channels: u32::try_from(par.ch_layout().nb_channels).unwrap_or(0),
                }
            } else {
                StreamKind::Other
            };
            // SAFETY: `avcodec_get_name` accepts any codec id and returns a static, NUL-terminated
            // string ("unknown_codec" for ids it does not know).
            let codec = unsafe { std::ffi::CStr::from_ptr(ffi::avcodec_get_name(par.codec_id)) }
                .to_string_lossy()
                .into_owned();
            StreamInfo {
                index,
                codec,
                kind,
                duration: timestamp_to_time(
                    stream.duration,
                    stream.time_base.num,
                    stream.time_base.den,
                ),
            }
        })
        .collect();

    Ok(MediaInfo {
        format: input.iformat().name().to_string_lossy().into_owned(),
        duration: timestamp_to_time(input.duration, 1, ffi::AV_TIME_BASE as i32),
        streams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_time_bases_exactly() {
        // 1 s in the MP4 video time base of the fixture and in microseconds (AV_TIME_BASE).
        assert_eq!(
            timestamp_to_time(12_800, 1, 12_800),
            Some(Time::from_seconds(1))
        );
        assert_eq!(
            timestamp_to_time(1_000_000, 1, 1_000_000),
            Some(Time::from_seconds(1))
        );
        // 48 kHz audio and NTSC frame durations are whole numbers of flicks.
        assert_eq!(
            timestamp_to_time(1, 1, 48_000),
            Some(Time::from_flicks(14_700))
        );
        assert_eq!(
            timestamp_to_time(1001, 1, 30_000),
            Some(Time::from_flicks(23_543_520))
        );
    }

    #[test]
    fn rounds_to_the_nearest_flick() {
        // 1 µs = 705.6 flicks.
        assert_eq!(
            timestamp_to_time(1, 1, 1_000_000),
            Some(Time::from_flicks(706))
        );
        assert_eq!(
            timestamp_to_time(-1, 1, 1_000_000),
            Some(Time::from_flicks(-706))
        );
    }

    #[test]
    fn rejects_missing_and_invalid_values() {
        assert_eq!(timestamp_to_time(ffi::AV_NOPTS_VALUE, 1, 1000), None);
        assert_eq!(timestamp_to_time(1, 1, 0), None);
        assert_eq!(timestamp_to_time(1, 0, 1), None);
        assert_eq!(timestamp_to_time(i64::MAX, 1, 1), None);
    }
}
