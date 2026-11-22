//! Software decoding for jobs (probe, and later previews), and the VFR timing rule.
//!
//! Nothing locates frames by index: every decoded frame carries its own timestamp, and the frame
//! shown for media time `t` is the last one with `pts <= t` (sample and hold, design §5). This
//! keeps variable-frame-rate phone footage in sync with its audio.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use app_core::Time;
use app_core::media::MediaInfo;
use rsmpeg::avcodec::{AVCodecContext, AVPacket};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::error::{MediaError, MediaErrorKind};
use super::input::{Input, map_error};
use super::probe::{codec_name, decoder_for, par, ts_to_time};

/// Threads per decoder in import and preview jobs (design §5).
const JOB_DECODER_THREADS: i32 = 2;

pub(crate) fn open_decoder(stream: &ffi::AVStream) -> Result<AVCodecContext, MediaError> {
    let codec = decoder_for(stream).ok_or_else(|| {
        MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("no decoder for {}", codec_name(par(stream).codec_id)),
        )
    })?;
    let mut ctx = AVCodecContext::new(&codec);
    // SAFETY: `ctx` is a fresh decoder context and `codecpar` belongs to an opened stream.
    let ret = unsafe { ffi::avcodec_parameters_to_context(ctx.as_mut_ptr(), stream.codecpar) };
    if ret < 0 {
        return Err(map_error(ret, "decoder parameters"));
    }
    ctx.set_pkt_timebase(stream.time_base);
    // SAFETY: setting a plain field before `avcodec_open2`.
    unsafe { (*ctx.as_mut_ptr()).thread_count = JOB_DECODER_THREADS };
    ctx.open(None).map_err(|err| {
        MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!(
                "cannot open the {} decoder: {err}",
                codec_name(par(stream).codec_id)
            ),
        )
    })?;
    Ok(ctx)
}

/// Sends `packet` (or flushes with `None`) and returns a decoded frame if one is ready.
pub(crate) fn send_and_receive(
    decoder: &mut AVCodecContext,
    packet: Option<&AVPacket>,
) -> Result<Option<AVFrame>, MediaError> {
    match decoder.send_packet(packet) {
        Ok(()) | Err(RsmpegError::DecoderFullError) | Err(RsmpegError::DecoderFlushedError) => {}
        Err(RsmpegError::SendPacketError(code)) => return Err(map_error(code, "decode")),
        Err(err) => return Err(MediaError::new(MediaErrorKind::Damaged, err.to_string())),
    }
    match decoder.receive_frame() {
        Ok(frame) => Ok(Some(frame)),
        Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => Ok(None),
        Err(RsmpegError::ReceiveFrameError(code)) => Err(map_error(code, "decode")),
        Err(err) => Err(MediaError::new(MediaErrorKind::Damaged, err.to_string())),
    }
}

/// Media times (container time minus `info.origin`) of every frame of the video stream of `info`,
/// in presentation order, decoded in software. For tests and diagnostics: it decodes the whole
/// stream.
pub fn video_frame_times(path: &Path, info: &MediaInfo) -> Result<Vec<Time>, MediaError> {
    let Some(video) = &info.video else {
        return Err(MediaError::new(
            MediaErrorKind::UnsupportedFormat,
            "no video stream",
        ));
    };
    let index = video.stream_index as usize;
    let cancel = AtomicBool::new(false);
    let mut input = Input::open(path, &cancel)?;
    let stream = *input
        .streams()
        .get(index)
        .ok_or_else(|| MediaError::new(MediaErrorKind::Damaged, "the video stream is gone"))?;
    let time_base = stream.time_base;
    let mut decoder = open_decoder(stream)?;
    let mut times = Vec::new();
    let mut push = |frame: &AVFrame| {
        if let Some(t) = ts_to_time(frame.best_effort_timestamp, time_base) {
            times.push(t - info.origin);
        }
    };
    while let Some(packet) = input.read_packet()? {
        if packet.stream_index as usize != index {
            continue;
        }
        decoder.send_packet(Some(&packet)).ok();
        while let Ok(frame) = decoder.receive_frame() {
            push(&frame);
        }
    }
    decoder.send_packet(None).ok();
    while let Ok(frame) = decoder.receive_frame() {
        push(&frame);
    }
    Ok(times)
}

/// The frame shown at media time `t`: the last frame whose time is `<= t` (sample and hold).
/// `times` is in presentation order. `None` before the first frame.
pub fn frame_index_at(times: &[Time], t: Time) -> Option<usize> {
    times.partition_point(|&frame| frame <= t).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_and_hold_picks_the_last_frame_at_or_before_t() {
        let times = [0, 10, 40, 50].map(Time::from_flicks);
        let at = |t| frame_index_at(&times, Time::from_flicks(t));
        assert_eq!(at(-1), None);
        assert_eq!(at(0), Some(0));
        assert_eq!(at(9), Some(0));
        assert_eq!(at(10), Some(1));
        assert_eq!(at(39), Some(1));
        assert_eq!(at(45), Some(2));
        assert_eq!(at(1000), Some(3));
        assert_eq!(frame_index_at(&[], Time::ZERO), None);
    }
}
