//! Sequential and seeking video decode for preview jobs (design §5).
//!
//! Frames carry their own timestamps; the frame for media time `t` is the last one with
//! `pts <= t` (sample and hold), so VFR sources stay right. Software decode only.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use app_core::Time;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::decode::open_decoder;
use super::error::{MediaError, MediaErrorKind};
use super::input::{Input, map_error};
use super::probe::ts_to_time;

/// Frames dropped while waiting for a keyframe after a seek, before taking whatever comes.
const MAX_SKIPPED_TO_KEY: usize = 600;

/// Undecodable packets tolerated in a row before the stream counts as damaged.
const MAX_BAD_PACKETS: usize = 64;

/// One open video stream of a file, decoding forward from wherever it was last sought to.
pub(crate) struct VideoReader<'c> {
    path: PathBuf,
    cancel: &'c AtomicBool,
    input: Input<'c>,
    decoder: AVCodecContext,
    index: usize,
    time_base: ffi::AVRational,
    origin: Time,
    /// A frame read past the last requested time, handed out first next time.
    held: Option<(Option<Time>, AVFrame)>,
    /// The decoder was flushed with an empty packet (end of file).
    flushed: bool,
    /// Frames before the first keyframe after a seek are dropped: a seek in a container without
    /// an exact index can land inside a GOP, and those frames miss their references.
    need_key: bool,
    skipped: usize,
}

impl std::fmt::Debug for VideoReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoReader")
            .field("path", &self.path)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

fn cancelled() -> MediaError {
    MediaError::new(MediaErrorKind::Cancelled, "cancelled")
}

impl<'c> VideoReader<'c> {
    /// Opens stream `index` of `path`. Media time is container time minus `origin`.
    pub(crate) fn open(
        path: &Path,
        index: u32,
        origin: Time,
        cancel: &'c AtomicBool,
    ) -> Result<Self, MediaError> {
        Self::open_with(path, index, origin, cancel, open_decoder)
    }

    /// [`open`](Self::open) with the decoder made by `open_decoder` (e.g. a hardware decoder,
    /// `media::d3d12`).
    pub(crate) fn open_with(
        path: &Path,
        index: u32,
        origin: Time,
        cancel: &'c AtomicBool,
        open_decoder: impl FnOnce(&ffi::AVStream) -> Result<AVCodecContext, MediaError>,
    ) -> Result<Self, MediaError> {
        let input = Input::open(path, cancel)?;
        let index = index as usize;
        let stream = *input
            .streams()
            .get(index)
            .ok_or_else(|| MediaError::new(MediaErrorKind::Damaged, "the video stream is gone"))?;
        let time_base = stream.time_base;
        let decoder = open_decoder(stream)?;
        Ok(Self {
            path: path.to_owned(),
            cancel,
            input,
            decoder,
            index,
            time_base,
            origin,
            held: None,
            flushed: false,
            need_key: false,
            skipped: 0,
        })
    }

    /// The decoder (e.g. to ask `media::d3d12::hw_active` after an error).
    pub(crate) fn decoder(&self) -> &AVCodecContext {
        &self.decoder
    }

    /// Which frames the decoder skips from the next packet on (`AVCodecContext::skip_frame`):
    /// `AVDISCARD_DEFAULT`, `AVDISCARD_NONREF` (catch-up) or `AVDISCARD_NONKEY` (keyframes only).
    pub(crate) fn set_skip_frame(&mut self, discard: ffi::AVDiscard) {
        // SAFETY: an opened decoder context; `skip_frame` is read by the decoder per frame and
        // may change between packets.
        unsafe { (*self.decoder.as_mut_ptr()).skip_frame = discard };
    }

    /// The next decoded frame and its media time (`None` if it has no timestamp), or `None` at
    /// the end of the stream.
    pub(crate) fn next_frame(&mut self) -> Result<Option<(Option<Time>, AVFrame)>, MediaError> {
        if let Some(held) = self.held.take() {
            return Ok(Some(held));
        }
        let mut bad_packets = 0;
        loop {
            match self.decoder.receive_frame() {
                Ok(frame) => {
                    if self.need_key {
                        let key = frame.flags & ffi::AV_FRAME_FLAG_KEY as i32 != 0;
                        self.skipped += 1;
                        if !key && self.skipped <= MAX_SKIPPED_TO_KEY {
                            continue;
                        }
                        self.need_key = false;
                    }
                    let time = ts_to_time(frame.best_effort_timestamp, self.time_base)
                        .map(|t| t - self.origin);
                    return Ok(Some((time, frame)));
                }
                Err(RsmpegError::DecoderDrainError) => {}
                Err(RsmpegError::DecoderFlushedError) => return Ok(None),
                Err(RsmpegError::ReceiveFrameError(code)) => return Err(map_error(code, "decode")),
                Err(err) => return Err(MediaError::new(MediaErrorKind::Damaged, err.to_string())),
            }
            if self.flushed {
                return Ok(None);
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            match self.input.read_packet()? {
                Some(packet) if packet.stream_index as usize == self.index => {
                    match self.decoder.send_packet(Some(&packet)) {
                        Ok(()) => bad_packets = 0,
                        // Damaged packets in the middle of a file: skip them, as players do.
                        Err(err) => {
                            bad_packets += 1;
                            if bad_packets > MAX_BAD_PACKETS {
                                return Err(MediaError::new(
                                    MediaErrorKind::Damaged,
                                    format!("too many undecodable packets: {err}"),
                                ));
                            }
                        }
                    }
                }
                Some(_) => {}
                None => {
                    self.decoder.send_packet(None).ok();
                    self.flushed = true;
                }
            }
        }
    }

    /// Moves to the keyframe at or before media time `t`. Containers that cannot seek (some
    /// GIFs) are reopened and read from the start.
    pub(crate) fn seek(&mut self, t: Time) -> Result<(), MediaError> {
        let ts = (t + self.origin).to_ticks(
            app_core::Rational::new(i64::from(self.time_base.num), i64::from(self.time_base.den))
                .map_err(|_| MediaError::new(MediaErrorKind::Damaged, "invalid time base"))?,
        );
        self.seek_ticks(ts)
    }

    /// Moves to the keyframe whose timestamp is at or before media time `t`, rounding `t` down
    /// to the stream's time base (not to the nearest tick like [`seek`](Self::seek)): a reverse
    /// window that must start strictly before a frame never lands on that frame.
    pub(crate) fn seek_floor(&mut self, t: Time) -> Result<(), MediaError> {
        let (num, den) = (
            i128::from(self.time_base.num),
            i128::from(self.time_base.den),
        );
        if num <= 0 || den <= 0 {
            return Err(MediaError::new(
                MediaErrorKind::Damaged,
                "invalid time base",
            ));
        }
        let flicks = i128::from((t + self.origin).flicks());
        let ts = (flicks * den).div_euclid(num * i128::from(app_core::FLICKS_PER_SECOND));
        self.seek_ticks(i64::try_from(ts).unwrap_or(if ts < 0 { i64::MIN } else { i64::MAX }))
    }

    fn seek_ticks(&mut self, ts: i64) -> Result<(), MediaError> {
        // SAFETY: an opened input context and a valid stream index.
        let ret = unsafe {
            ffi::avformat_seek_file(
                self.input.as_mut_ptr(),
                self.index as i32,
                i64::MIN,
                ts,
                ts,
                0,
            )
        };
        if ret < 0 {
            tracing::debug!(path = %self.path.display(), "seek failed; reading from the start");
            return self.rewind();
        }
        self.reset_decoder();
        Ok(())
    }

    /// Reopens the file and reads from its start.
    pub(crate) fn rewind(&mut self) -> Result<(), MediaError> {
        self.input = Input::open(&self.path, self.cancel)?;
        self.reset_decoder();
        Ok(())
    }

    fn reset_decoder(&mut self) {
        // SAFETY: an opened decoder context.
        unsafe { ffi::avcodec_flush_buffers(self.decoder.as_mut_ptr()) };
        self.held = None;
        self.flushed = false;
        self.need_key = true;
        self.skipped = 0;
    }

    /// Seeks to `t` and returns the frame there ([`frame_at`](Self::frame_at)). If the seek
    /// lands where no frame follows (an imprecise index in some MPEG-PS and AVI files), reads
    /// from the start instead.
    pub(crate) fn seek_frame_at(
        &mut self,
        t: Time,
        approx: Option<Time>,
    ) -> Result<(AVFrame, bool), MediaError> {
        self.seek(t)?;
        match self.frame_at(t, approx) {
            Err(err) if err.kind == MediaErrorKind::Damaged => {
                tracing::debug!(path = %self.path.display(), %err, "no frame after the seek; reading from the start");
                self.rewind()?;
                self.frame_at(t, None).map(|(frame, _)| (frame, true))
            }
            other => other,
        }
    }

    /// The frame shown at media time `t`, decoding forward from the current position.
    ///
    /// With `approx`, right after a [`seek`](Self::seek): the first frame (the keyframe) is taken
    /// if it is at most `approx` before `t`, which keeps filmstrips of long files fast. Returns
    /// the frame and whether frames past the first had to be decoded.
    pub(crate) fn frame_at(
        &mut self,
        t: Time,
        approx: Option<Time>,
    ) -> Result<(AVFrame, bool), MediaError> {
        let mut best: Option<AVFrame> = None;
        let mut decoded = 0usize;
        while let Some((time, frame)) = self.next_frame()? {
            decoded += 1;
            let at_or_before = time.is_none_or(|time| time <= t);
            if at_or_before {
                if decoded == 1
                    && approx.is_some_and(|approx| time.is_none_or(|time| t - time <= approx))
                {
                    return Ok((frame, false));
                }
                best = Some(frame);
                continue;
            }
            match best {
                // The stream starts after `t`: its first frame is the closest.
                None => return Ok((frame, decoded > 1)),
                Some(best) => {
                    self.held = Some((time, frame));
                    return Ok((best, decoded > 2));
                }
            }
        }
        best.map(|frame| (frame, decoded > 1)).ok_or_else(|| {
            MediaError::new(MediaErrorKind::Damaged, "the video stream has no frames")
        })
    }
}
