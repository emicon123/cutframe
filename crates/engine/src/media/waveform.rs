//! Waveforms: a mono mix-down of the default audio stream as min/max pairs per 10 ms bucket
//! (design §4 "Waveform body").

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use app_core::media::{MediaInfo, WAVEFORM_BUCKETS_PER_SECOND};
use app_core::{FLICKS_PER_SECOND, Time};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::decode::open_decoder;
use super::error::{MediaError, MediaErrorKind};
use super::input::{Input, map_error};
use super::probe::ts_to_time;

/// Magic bytes at the start of a waveform body.
pub const WAVEFORM_MAGIC: &[u8; 4] = b"CFWF";
/// Version of the waveform body layout.
pub const WAVEFORM_VERSION: u16 = 1;
/// Bytes before the buckets: magic, version, channels, buckets per second, bucket count.
pub const WAVEFORM_HEADER_BYTES: usize = 16;

/// Min/max of a mono mix-down, scaled to ±127, [`WAVEFORM_BUCKETS_PER_SECOND`] buckets per second
/// of media time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    pub buckets: Vec<(i8, i8)>,
}

impl Waveform {
    /// Little endian: `CFWF`, `u16` version, `u16` channels (1), `u32` buckets per second,
    /// `u32` bucket count, then `count x (i8 min, i8 max)`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(WAVEFORM_HEADER_BYTES + self.buckets.len() * 2);
        out.extend_from_slice(WAVEFORM_MAGIC);
        out.extend_from_slice(&WAVEFORM_VERSION.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&WAVEFORM_BUCKETS_PER_SECOND.to_le_bytes());
        out.extend_from_slice(&(self.buckets.len() as u32).to_le_bytes());
        for &(min, max) in &self.buckets {
            out.extend([min as u8, max as u8]);
        }
        out
    }
}

/// Buckets covering `duration` (rounded up, at least one).
pub fn bucket_count(duration: Time) -> usize {
    let flicks = i128::from(duration.flicks().max(0));
    let per_second = i128::from(WAVEFORM_BUCKETS_PER_SECOND);
    let second = i128::from(FLICKS_PER_SECOND);
    ((flicks * per_second + second - 1) / second).max(1) as usize
}

/// Accumulates mono samples into buckets.
#[derive(Debug)]
struct Buckets {
    sample_rate: u64,
    min: Vec<f32>,
    max: Vec<f32>,
    /// Index of the next sample, counted from media time zero.
    position: u64,
}

impl Buckets {
    fn new(count: usize, sample_rate: u32) -> Self {
        Self {
            sample_rate: u64::from(sample_rate.max(1)),
            min: vec![0.0; count],
            max: vec![0.0; count],
            position: 0,
        }
    }

    fn push(&mut self, sample: f32) {
        let bucket = self.position * u64::from(WAVEFORM_BUCKETS_PER_SECOND) / self.sample_rate;
        self.position += 1;
        let Some(min) = self.min.get_mut(bucket as usize) else {
            return;
        };
        *min = min.min(sample);
        let max = &mut self.max[bucket as usize];
        *max = max.max(sample);
    }

    fn finish(self) -> Waveform {
        let scale = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
        Waveform {
            buckets: self
                .min
                .iter()
                .zip(&self.max)
                .map(|(&min, &max)| (scale(min), scale(max)))
                .collect(),
        }
    }
}

/// Sample `i` of channel `ch` of a decoded audio frame as -1..=1.
fn sample(
    frame: &AVFrame,
    format: ffi::AVSampleFormat,
    channels: usize,
    ch: usize,
    i: usize,
) -> f32 {
    // SAFETY: plain lookups on a valid sample format.
    let planar = unsafe { ffi::av_sample_fmt_is_planar(format) } != 0;
    // SAFETY: as above.
    let size = unsafe { ffi::av_get_bytes_per_sample(format) } as usize;
    let (plane, offset) = if planar {
        (ch, i * size)
    } else {
        (0, (i * channels + ch) * size)
    };
    let data = if plane < 8 {
        frame.data[plane]
    } else {
        // SAFETY: frames with more than 8 planes keep all of them in `extended_data`.
        unsafe { *frame.extended_data.add(plane) }
    };
    // SAFETY: `i < nb_samples` and `ch < channels`, so the offset is inside the plane.
    unsafe {
        let p = data.add(offset);
        match format {
            ffi::AV_SAMPLE_FMT_U8 | ffi::AV_SAMPLE_FMT_U8P => (f32::from(*p) - 128.0) / 128.0,
            ffi::AV_SAMPLE_FMT_S16 | ffi::AV_SAMPLE_FMT_S16P => {
                f32::from(p.cast::<i16>().read_unaligned()) / 32_768.0
            }
            ffi::AV_SAMPLE_FMT_S32 | ffi::AV_SAMPLE_FMT_S32P => {
                p.cast::<i32>().read_unaligned() as f32 / 2_147_483_648.0
            }
            ffi::AV_SAMPLE_FMT_S64 | ffi::AV_SAMPLE_FMT_S64P => {
                p.cast::<i64>().read_unaligned() as f32 / 9_223_372_036_854_775_808.0
            }
            ffi::AV_SAMPLE_FMT_FLT | ffi::AV_SAMPLE_FMT_FLTP => p.cast::<f32>().read_unaligned(),
            ffi::AV_SAMPLE_FMT_DBL | ffi::AV_SAMPLE_FMT_DBLP => {
                p.cast::<f64>().read_unaligned() as f32
            }
            _ => 0.0,
        }
    }
}

/// Decodes the default audio stream of `path` (described by `info`) once and reduces it.
pub fn waveform(
    path: &Path,
    info: &MediaInfo,
    cancel: &AtomicBool,
) -> Result<Waveform, MediaError> {
    let audio = info
        .audio
        .first()
        .ok_or_else(|| MediaError::new(MediaErrorKind::UnsupportedFormat, "no audio stream"))?;
    let duration = info
        .duration
        .ok_or_else(|| MediaError::new(MediaErrorKind::UnsupportedFormat, "no duration"))?;
    let mut input = Input::open(path, cancel)?;
    let index = audio.stream_index as usize;
    let stream = *input
        .streams()
        .get(index)
        .ok_or_else(|| MediaError::new(MediaErrorKind::Damaged, "the audio stream is gone"))?;
    let time_base = stream.time_base;
    let mut decoder = open_decoder(stream)?;
    let mut buckets = Buckets::new(bucket_count(duration), audio.sample_rate);
    let mut started = false;

    let mut take = |frame: &AVFrame, buckets: &mut Buckets| {
        let channels = frame.ch_layout.nb_channels.max(1) as usize;
        let rate = frame.sample_rate.max(1) as u64;
        if !started {
            started = true;
            // Audio that starts after media time zero (later than the video) begins with silence.
            if let Some(start) = ts_to_time(frame.best_effort_timestamp, time_base) {
                let offset = (start - info.origin).flicks().max(0);
                buckets.position =
                    (i128::from(offset) * i128::from(rate) / i128::from(FLICKS_PER_SECOND)) as u64;
            }
        }
        buckets.sample_rate = rate;
        let format = frame.format;
        for i in 0..frame.nb_samples.max(0) as usize {
            let sum: f32 = (0..channels)
                .map(|ch| sample(frame, format, channels, ch, i))
                .sum();
            buckets.push(sum / channels as f32);
        }
    };

    let mut flushed = false;
    loop {
        match decoder.receive_frame() {
            Ok(frame) => {
                take(&frame, &mut buckets);
                continue;
            }
            Err(RsmpegError::DecoderDrainError) => {}
            Err(RsmpegError::DecoderFlushedError) => break,
            Err(RsmpegError::ReceiveFrameError(code)) => return Err(map_error(code, "decode")),
            Err(err) => return Err(MediaError::new(MediaErrorKind::Damaged, err.to_string())),
        }
        if flushed {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::new(MediaErrorKind::Cancelled, "cancelled"));
        }
        match input.read_packet()? {
            Some(packet) if packet.stream_index as usize == index => {
                // A damaged packet in the middle: skip it.
                decoder.send_packet(Some(&packet)).ok();
            }
            Some(_) => {}
            None => {
                decoder.send_packet(None).ok();
                flushed = true;
            }
        }
    }
    Ok(buckets.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_layout() {
        let bytes = Waveform {
            buckets: vec![(-127, 127), (0, 5)],
        }
        .to_bytes();
        assert_eq!(&bytes[..4], b"CFWF");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 1);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 100);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
        assert_eq!(&bytes[16..], &[0x81, 0x7f, 0, 5]);
    }

    #[test]
    fn buckets_cover_the_duration() {
        assert_eq!(bucket_count(Time::ZERO), 1);
        assert_eq!(bucket_count(Time::from_seconds(1)), 100);
        assert_eq!(
            bucket_count(Time::from_flicks(FLICKS_PER_SECOND / 100 + 1)),
            2
        );
        assert_eq!(bucket_count(Time::from_seconds(3600)), 360_000);
    }

    #[test]
    fn samples_land_in_their_bucket() {
        let mut buckets = Buckets::new(2, 400);
        for i in 0..8 {
            buckets.push(if i == 5 { -0.5 } else { 0.25 });
        }
        let waveform = buckets.finish();
        assert_eq!(waveform.buckets, [(0, 32), (-64, 32)]);
    }
}
