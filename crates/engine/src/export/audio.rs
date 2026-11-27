//! Export audio (export.md §4.5): the timeline mixed by the shared [`mix_block`] at the project
//! sample rate, in blocks of [`MIX_CHUNK_FRAMES`] (the AAC frame), encoded as AAC-LC stereo at
//! [`AUDIO_BITRATE`] with FFmpeg's native `aac` encoder (LGPL).
//!
//! - **Timing.** Block `i` starts at sample `i x 1024`, converted to flicks exactly by
//!   [`sample_offset`], never accumulated; `pts` is the sample index (`1 / sample_rate`).
//! - **Length.** Exactly `samples` samples (the plan's count, export.md §4.1, which ends with the
//!   last video frame); the last block is short. The encoder's priming delay (1024 samples)
//!   becomes the MP4 edit list (FFmpeg's default), so audio and video start together.
//! - **Silent timelines** still produce an AAC track (of silence).
//! - **Interleaving.** After video frame `k` the run loop calls
//!   [`AudioEncoder::encode_until`] with `(k + 1) x F`; only whole blocks are encoded before
//!   [`AudioEncoder::finish`], so at most about one frame of each stream is buffered.
//!
//! Speed, reverse, pitch preservation and the 4x mute come through `mix_block` and the
//! [`AudioSources`] voices, as in playback (clip-speed-audio.md §5.3).
//!
//! Threading: create and use an [`AudioEncoder`] on the export thread; it owns its readers.

use std::sync::Arc;

use app_core::{Project, Time};
use rsmpeg::UnsafeDerefMut;
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avutil::{AVChannelLayout, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::mux::StreamError;
use crate::audio::mix::MIX_CHUNK_FRAMES;
use crate::audio::{AudioSources, mix_block, sample_offset, samples_before};

/// AAC bit rate in bits per second (YouTube's recommendation for stereo).
pub const AUDIO_BITRATE: i64 = 384_000;

/// The AAC encoder of one export with its own audio readers.
pub struct AudioEncoder {
    project: Arc<Project>,
    sources: AudioSources,
    sample_rate: u32,
    /// Samples per channel of the whole export.
    total: u64,
    /// The next sample to encode.
    next: u64,
    context: AVCodecContext,
    frame: AVFrame,
    /// One block, stereo interleaved.
    mix: Vec<f32>,
    flushed: bool,
}

impl std::fmt::Debug for AudioEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEncoder")
            .field("sample_rate", &self.sample_rate)
            .field("total", &self.total)
            .field("next", &self.next)
            .field("flushed", &self.flushed)
            .finish_non_exhaustive()
    }
}

impl AudioEncoder {
    /// Opens the AAC encoder for `samples` samples of `project`'s timeline at `sample_rate`
    /// (the project rate). The encoder has `AV_CODEC_FLAG_GLOBAL_HEADER` set (MP4).
    pub fn new(project: Arc<Project>, sample_rate: u32, samples: u64) -> Result<Self, StreamError> {
        let codec = AVCodec::find_encoder_by_name(c"aac")
            .ok_or_else(|| StreamError::Encoder("FFmpeg has no native aac encoder".into()))?;
        let rate = i32::try_from(sample_rate)
            .ok()
            .filter(|&r| r > 0)
            .ok_or_else(|| StreamError::Internal(format!("sample rate {sample_rate}")))?;
        let mut context = AVCodecContext::new(&codec);
        context.set_sample_fmt(ffi::AV_SAMPLE_FMT_FLTP);
        context.set_sample_rate(rate);
        context.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
        context.set_bit_rate(AUDIO_BITRATE);
        context.set_time_base(ffi::AVRational { num: 1, den: rate });
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
        // SAFETY: plain field write on a valid, unopened codec context.
        unsafe { context.deref_mut().profile = ffi::AV_PROFILE_AAC_LOW as i32 };
        context
            .open(None)
            .map_err(|err| StreamError::Encoder(format!("open aac: {err}")))?;
        if usize::try_from(context.frame_size).ok() != Some(MIX_CHUNK_FRAMES) {
            return Err(StreamError::Encoder(format!(
                "aac frame size is {}, not {MIX_CHUNK_FRAMES}",
                context.frame_size
            )));
        }
        let mut frame = AVFrame::new();
        frame.set_format(ffi::AV_SAMPLE_FMT_FLTP);
        frame.set_nb_samples(MIX_CHUNK_FRAMES as i32);
        frame.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
        frame.set_sample_rate(rate);
        frame
            .alloc_buffer()
            .map_err(|err| StreamError::Internal(format!("audio frame: {err}")))?;
        Ok(Self {
            project,
            sources: AudioSources::new(),
            sample_rate,
            total: samples,
            next: 0,
            context,
            frame,
            mix: vec![0.0; 2 * MIX_CHUNK_FRAMES],
            flushed: false,
        })
    }

    /// The opened encoder, for [`super::mux::Muxer::create`].
    pub fn context(&self) -> &AVCodecContext {
        &self.context
    }

    /// Samples per channel handed to the encoder so far.
    pub fn encoded_samples(&self) -> u64 {
        self.next
    }

    /// Samples per channel of the whole export.
    pub fn total_samples(&self) -> u64 {
        self.total
    }

    /// Encodes the whole blocks that end at or before timeline time `until` (and before the
    /// end of the export) and hands every packet the encoder returns to `sink` (normally
    /// [`super::mux::Muxer::write_audio`]).
    pub fn encode_until(
        &mut self,
        until: Time,
        sink: &mut dyn FnMut(AVPacket) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        let target = u64::try_from(samples_before(until, self.sample_rate))
            .unwrap_or(0)
            .min(self.total);
        while self.next + MIX_CHUNK_FRAMES as u64 <= target {
            self.encode_block(MIX_CHUNK_FRAMES, sink)?;
        }
        Ok(())
    }

    /// Encodes the rest of the export (the last block is short), flushes the encoder and hands
    /// the remaining packets to `sink`. Further calls do nothing.
    pub fn finish(
        &mut self,
        sink: &mut dyn FnMut(AVPacket) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        if self.flushed {
            return Ok(());
        }
        while self.next < self.total {
            let n = (self.total - self.next).min(MIX_CHUNK_FRAMES as u64) as usize;
            self.encode_block(n, sink)?;
        }
        self.flushed = true;
        send(&mut self.context, None, sink)
    }

    /// Mixes and encodes `frames` samples from `self.next`.
    fn encode_block(
        &mut self,
        frames: usize,
        sink: &mut dyn FnMut(AVPacket) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        let start = sample_offset(self.next as i64, self.sample_rate);
        mix_block(
            &self.project,
            start,
            frames,
            self.sample_rate,
            &mut self.sources,
            &mut self.mix,
        );
        self.frame
            .make_writable()
            .map_err(|err| StreamError::Internal(format!("audio frame: {err}")))?;
        self.frame.set_nb_samples(frames as i32);
        self.frame.set_pts(self.next as i64);
        let data = self.frame.data;
        for (channel, plane) in data.iter().take(2).enumerate() {
            // SAFETY: the frame is writable, `fltp` stereo, allocated for `MIX_CHUNK_FRAMES >=
            // frames` samples per plane, so `data[0]` and `data[1]` hold `frames` aligned `f32`s
            // each; the two planes do not overlap.
            let out = unsafe { std::slice::from_raw_parts_mut(plane.cast::<f32>(), frames) };
            for (sample, frame) in out.iter_mut().zip(self.mix.as_chunks::<2>().0) {
                *sample = frame[channel];
            }
        }
        self.next += frames as u64;
        send(&mut self.context, Some(&self.frame), sink)
    }
}

/// Sends `frame` (or the flush) to `context` and drains every packet into `sink`.
fn send(
    context: &mut AVCodecContext,
    frame: Option<&AVFrame>,
    sink: &mut dyn FnMut(AVPacket) -> Result<(), StreamError>,
) -> Result<(), StreamError> {
    context
        .send_frame(frame)
        .map_err(|err| StreamError::Encoder(format!("aac send_frame: {err}")))?;
    loop {
        match context.receive_packet() {
            Ok(packet) => sink(packet)?,
            Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => {
                return Ok(());
            }
            Err(err) => return Err(StreamError::Encoder(format!("aac receive_packet: {err}"))),
        }
    }
}
