//! Audio readers (playback.md §6.4): one FFmpeg demuxer, decoder and resampler per voice, with
//! sample-accurate seeks, and [`AudioSources`], the [`PcmProvider`] that assigns readers to
//! voices the way video readers are assigned to clips (§7): a reader that reaches the next
//! clip's source time by decoding forward continues (a split point plays without a seek), and
//! [`AudioSources::prefetch`] pre-seeks an idle reader for a jump cut within the lookahead.
//!
//! Both are plain owned values: construct and use them on any one thread (the feeder, the export
//! thread). They allocate and do I/O, so never on the real-time callback.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use app_core::media::MediaId;
use app_core::model::{ClipId, ClipSource};
use app_core::{FLICKS_PER_SECOND, Project, Time};
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avutil::{AVChannelLayout, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;

use super::mix::{PcmProvider, Voice};
use super::{sample_offset, samples_before};
use crate::media::decode::open_decoder_with_threads;
use crate::media::input::{Input, map_error};
use crate::media::probe::ts_to_time;
use crate::media::{MediaError, MediaErrorKind};

/// Audio readers read local files and are never cancelled.
static NO_CANCEL: AtomicBool = AtomicBool::new(false);

/// A seek lands this far before the target, so decoders with inter-frame state (AAC overlap,
/// MP3 bit reservoir, Opus pre-skip) have settled when the target sample comes.
const SEEK_PREROLL: Time = Time(FLICKS_PER_SECOND / 5);

/// Undecodable packets tolerated in a row before the stream counts as damaged.
const MAX_BAD_PACKETS: usize = 64;

/// A reader within this much before the requested time decodes forward instead of seeking.
const FORWARD_SKIP: Time = Time(FLICKS_PER_SECOND / 2);

/// Readers that no voice used for this long close (playback.md §7).
const IDLE_CLOSE: Duration = Duration::from_secs(5);

/// A media that failed to open is tried again after this long (a USB drive plugged back in).
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// At most this many open readers per [`AudioSources`].
const MAX_READERS: usize = 16;

/// Converts decoded frames to interleaved stereo `f32` at the output rate.
struct Resampler {
    swr: SwrContext,
    in_rate: i32,
    in_format: i32,
    in_channels: i32,
    /// Mono sources are resampled as mono and played on both channels at full level (the
    /// default matrix would put them at -3 dB).
    mono: bool,
}

/// One audio stream of a media file, decoded forward from the last seek, resampled to stereo
/// `f32` at a fixed rate. Media time is container time minus `origin` (media-import.md §5).
pub struct AudioReader {
    path: PathBuf,
    input: Input<'static>,
    decoder: AVCodecContext,
    index: usize,
    time_base: ffi::AVRational,
    origin: Time,
    /// Media time of the stream's first sample (later than 0 when the audio starts after the
    /// video).
    stream_start: Time,
    sample_rate: u32,
    resampler: Option<Resampler>,
    /// Resampled stereo frames not read yet, from `fifo_read` on.
    fifo: Vec<f32>,
    fifo_read: usize,
    mono: Vec<f32>,
    /// Media time of output frame 0 after the last seek, and frames handed out since.
    anchor: Time,
    delivered: i64,
    /// After a seek: input before this media time is dropped.
    pending: Option<Time>,
    /// The seek landed past its target once and the file was read from the start.
    retried: bool,
    /// The decoder was flushed (end of file) / everything was handed out.
    flushed: bool,
    eof: bool,
    seeks: u64,
}

impl std::fmt::Debug for AudioReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioReader")
            .field("path", &self.path)
            .field("index", &self.index)
            .field("sample_rate", &self.sample_rate)
            .field("position", &self.position())
            .field("seeks", &self.seeks)
            .finish_non_exhaustive()
    }
}

impl AudioReader {
    /// Opens audio stream `stream_index` of `path`, resampling to `sample_rate`. Media time is
    /// container time minus `origin`. The reader is positioned at media time 0 without a seek.
    pub fn open(
        path: &Path,
        stream_index: u32,
        origin: Time,
        sample_rate: u32,
    ) -> Result<Self, MediaError> {
        let input = Input::open(path, &NO_CANCEL)?;
        let index = stream_index as usize;
        let stream = *input
            .streams()
            .get(index)
            .ok_or_else(|| MediaError::new(MediaErrorKind::Damaged, "the audio stream is gone"))?;
        if stream.codecpar.is_null()
            // SAFETY: `codecpar` of an opened stream is valid (checked for NULL first).
            || unsafe { (*stream.codecpar).codec_type } != ffi::AVMEDIA_TYPE_AUDIO
        {
            return Err(MediaError::new(
                MediaErrorKind::UnsupportedFormat,
                "not an audio stream",
            ));
        }
        let time_base = stream.time_base;
        let stream_start =
            ts_to_time(stream.start_time, time_base).map_or(Time::ZERO, |t| t - origin);
        let decoder = open_decoder_with_threads(stream, 1)?;
        Ok(Self {
            path: path.to_owned(),
            input,
            decoder,
            index,
            time_base,
            origin,
            stream_start,
            sample_rate: sample_rate.max(1),
            resampler: None,
            fifo: Vec::new(),
            fifo_read: 0,
            mono: Vec::new(),
            anchor: Time::ZERO,
            delivered: 0,
            pending: Some(Time::ZERO),
            retried: false,
            flushed: false,
            eof: false,
            seeks: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Media time of the next frame [`Self::read`] hands out.
    pub fn position(&self) -> Time {
        self.anchor + sample_offset(self.delivered, self.sample_rate)
    }

    /// Seeks done so far (tests count them: a split point plays without one).
    pub fn seeks(&self) -> u64 {
        self.seeks
    }

    /// Moves to media time `t`, sample-accurately: seeks to a packet before `t` (with preroll
    /// for the decoder), flushes the decoder and resampler, and drops decoded samples before `t`
    /// by their timestamps.
    pub fn seek(&mut self, t: Time) -> Result<(), MediaError> {
        self.seeks += 1;
        let target = (t - SEEK_PREROLL).max(Time::ZERO.min(t));
        let ts = self.ticks(target)?;
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
            tracing::debug!(path = %self.path.display(), "audio seek failed; reading from the start");
            self.input = Input::open(&self.path, &NO_CANCEL)?;
        }
        self.reset(t);
        Ok(())
    }

    /// Decodes until at least `frames` frames are buffered (a pre-seeked reader is then ready
    /// before its clip starts).
    pub fn prime(&mut self, frames: usize) -> Result<(), MediaError> {
        while self.buffered() < frames && !self.eof {
            self.decode_more()?;
        }
        Ok(())
    }

    /// Fills `out` (stereo interleaved) with the next `out.len() / 2` frames; silence past the
    /// end of the stream.
    pub fn read(&mut self, out: &mut [f32]) -> Result<(), MediaError> {
        let frames = out.len() / 2;
        self.prime(frames)?;
        let available = self.buffered().min(frames);
        let from = self.fifo_read;
        out[..2 * available].copy_from_slice(&self.fifo[from..from + 2 * available]);
        out[2 * available..].fill(0.0);
        self.fifo_read += 2 * available;
        self.delivered += frames as i64;
        if self.fifo_read >= 1 << 15 {
            self.fifo.drain(..self.fifo_read);
            self.fifo_read = 0;
        }
        Ok(())
    }

    /// Skips `frames` frames by decoding forward.
    pub fn discard(&mut self, mut frames: usize) -> Result<(), MediaError> {
        let mut scratch = [0.0f32; 2048];
        while frames > 0 {
            let n = frames.min(1024);
            self.read(&mut scratch[..2 * n])?;
            frames -= n;
        }
        Ok(())
    }

    fn buffered(&self) -> usize {
        (self.fifo.len() - self.fifo_read) / 2
    }

    fn ticks(&self, t: Time) -> Result<i64, MediaError> {
        let tb =
            app_core::Rational::new(i64::from(self.time_base.num), i64::from(self.time_base.den))
                .map_err(|_| MediaError::new(MediaErrorKind::Damaged, "invalid time base"))?;
        Ok((t + self.origin).to_ticks(tb))
    }

    fn reset(&mut self, t: Time) {
        // SAFETY: an opened decoder context.
        unsafe { ffi::avcodec_flush_buffers(self.decoder.as_mut_ptr()) };
        self.resampler = None;
        self.fifo.clear();
        self.fifo_read = 0;
        self.anchor = t;
        self.delivered = 0;
        self.pending = Some(t);
        self.retried = false;
        self.flushed = false;
        self.eof = false;
    }

    /// Decodes one more frame into the FIFO, or marks the end of the stream.
    fn decode_more(&mut self) -> Result<(), MediaError> {
        match self.next_frame()? {
            Some(frame) => self.take(&frame),
            None => {
                self.flush_resampler()?;
                self.eof = true;
                Ok(())
            }
        }
    }

    /// The next decoded frame, `None` at the end of the stream.
    fn next_frame(&mut self) -> Result<Option<AVFrame>, MediaError> {
        let mut bad_packets = 0;
        loop {
            match self.decoder.receive_frame() {
                Ok(frame) => return Ok(Some(frame)),
                Err(RsmpegError::DecoderDrainError) => {}
                Err(RsmpegError::DecoderFlushedError) => return Ok(None),
                Err(RsmpegError::ReceiveFrameError(code)) => return Err(map_error(code, "decode")),
                Err(err) => return Err(MediaError::new(MediaErrorKind::Damaged, err.to_string())),
            }
            if self.flushed {
                return Ok(None);
            }
            match self.input.read_packet()? {
                Some(packet) if packet.stream_index as usize == self.index => {
                    match self.decoder.send_packet(Some(&packet)) {
                        Ok(()) => bad_packets = 0,
                        Err(err) => {
                            bad_packets += 1;
                            if bad_packets > MAX_BAD_PACKETS {
                                return Err(MediaError::new(
                                    MediaErrorKind::Damaged,
                                    format!("too many undecodable audio packets: {err}"),
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

    /// Adds a decoded frame, aligned to the pending seek target if there is one.
    fn take(&mut self, frame: &AVFrame) -> Result<(), MediaError> {
        let in_rate = frame.sample_rate.max(1) as u32;
        let samples = frame.nb_samples.max(0) as usize;
        let mut skip = 0;
        if let Some(target) = self.pending {
            if let Some(start) = ts_to_time(frame.best_effort_timestamp, self.time_base) {
                let start = start - self.origin;
                let end = start + sample_offset(samples as i64, in_rate);
                if end <= target {
                    return Ok(()); // before the target
                }
                let tolerance = sample_offset(1, self.sample_rate);
                if start > target + tolerance {
                    if !self.retried && target >= self.stream_start + tolerance {
                        // The seek landed past the target (an inexact index): read from the start.
                        tracing::debug!(path = %self.path.display(), "audio seek overshot; reading from the start");
                        self.retried = true;
                        self.input = Input::open(&self.path, &NO_CANCEL)?;
                        // SAFETY: an opened decoder context.
                        unsafe { ffi::avcodec_flush_buffers(self.decoder.as_mut_ptr()) };
                        self.flushed = false;
                        return Ok(());
                    }
                    // The audio starts after the target: silence first.
                    let gap = samples_before(start - target, self.sample_rate) as usize;
                    self.fifo.resize(self.fifo.len() + 2 * gap, 0.0);
                } else if start < target {
                    let late = i128::from((target - start).flicks()) * i128::from(in_rate);
                    let fps = i128::from(FLICKS_PER_SECOND);
                    skip = usize::try_from((late + fps / 2) / fps)
                        .unwrap_or(0)
                        .min(samples);
                }
            }
            self.pending = None;
        }
        self.resample(frame, skip)
    }

    fn ensure_resampler(&mut self, frame: &AVFrame) -> Result<(), MediaError> {
        let channels = frame.ch_layout.nb_channels.max(1);
        let fits = self.resampler.as_ref().is_some_and(|r| {
            r.in_rate == frame.sample_rate
                && r.in_format == frame.format
                && r.in_channels == channels
        });
        if fits {
            return Ok(());
        }
        let mono = channels == 1;
        let out_layout = AVChannelLayout::from_nb_channels(if mono { 1 } else { 2 });
        // Decoders that leave the layout unspecified get the default layout of their channels.
        let default_in = AVChannelLayout::from_nb_channels(channels);
        let in_layout = if frame.ch_layout.order == ffi::AV_CHANNEL_ORDER_UNSPEC {
            &*default_in
        } else {
            &frame.ch_layout
        };
        let failed = |err: RsmpegError| {
            MediaError::new(
                MediaErrorKind::UnsupportedCodec,
                format!("cannot resample the audio: {err}"),
            )
        };
        let mut swr = SwrContext::new(
            &out_layout,
            ffi::AV_SAMPLE_FMT_FLT,
            self.sample_rate as i32,
            in_layout,
            frame.format,
            frame.sample_rate,
        )
        .map_err(failed)?;
        swr.init().map_err(failed)?;
        self.resampler = Some(Resampler {
            swr,
            in_rate: frame.sample_rate,
            in_format: frame.format,
            in_channels: channels,
            mono,
        });
        Ok(())
    }

    /// Resamples `frame` from sample `skip` on into the FIFO.
    fn resample(&mut self, frame: &AVFrame, skip: usize) -> Result<(), MediaError> {
        self.ensure_resampler(frame)?;
        let samples = frame.nb_samples.max(0) as usize;
        if skip >= samples {
            return Ok(());
        }
        let channels = frame.ch_layout.nb_channels.max(1) as usize;
        // SAFETY: plain lookups on the frame's valid sample format.
        let planar = unsafe { ffi::av_sample_fmt_is_planar(frame.format) } != 0;
        // SAFETY: as above.
        let bytes = unsafe { ffi::av_get_bytes_per_sample(frame.format) }.max(0) as usize;
        let planes = if planar { channels } else { 1 };
        let mut inputs = [ptr::null::<u8>(); 64];
        if planes > inputs.len() {
            return Err(MediaError::new(
                MediaErrorKind::UnsupportedFormat,
                "too many audio channels",
            ));
        }
        let offset = if planar {
            skip * bytes
        } else {
            skip * bytes * channels
        };
        for (plane, input) in inputs.iter_mut().enumerate().take(planes) {
            // SAFETY: `extended_data` holds one valid pointer per plane (all channels for planar
            // audio, one for packed), each with `nb_samples` samples; `skip < nb_samples`.
            *input = unsafe { (*frame.extended_data.add(plane)).add(offset).cast_const() };
        }
        let count = (samples - skip) as i32;
        // SAFETY: `inputs` holds `planes` valid plane pointers with `count` samples each.
        unsafe { self.convert(inputs.as_ptr(), count) }
    }

    fn flush_resampler(&mut self) -> Result<(), MediaError> {
        if self.resampler.is_none() {
            return Ok(());
        }
        // SAFETY: a NULL input with count 0 drains the resampler.
        unsafe { self.convert(ptr::null(), 0) }
    }

    /// Runs the resampler on `count` input samples at `inputs` (NULL and 0: drain) and appends
    /// the output to the FIFO.
    ///
    /// # Safety
    /// `inputs` is NULL with `count == 0`, or points at one valid pointer per input plane, each
    /// with `count` samples in the resampler's input format.
    unsafe fn convert(&mut self, inputs: *const *const u8, count: i32) -> Result<(), MediaError> {
        let resampler = self.resampler.as_mut().expect("ensure_resampler ran");
        let capacity = resampler.swr.get_out_samples(count).max(0) as usize + 32;
        let failed = |err: RsmpegError| {
            MediaError::new(MediaErrorKind::Damaged, format!("resampling failed: {err}"))
        };
        if resampler.mono {
            self.mono.resize(capacity, 0.0);
            let mut out = self.mono.as_mut_ptr().cast::<u8>();
            // SAFETY: `out` has room for `capacity` mono f32 samples; the inputs are valid as
            // the caller promises.
            let written = unsafe {
                resampler
                    .swr
                    .convert(&mut out, capacity as i32, inputs, count)
            }
            .map_err(failed)?
            .max(0) as usize;
            self.fifo.reserve(2 * written);
            for &s in &self.mono[..written] {
                self.fifo.extend([s, s]);
            }
        } else {
            let len = self.fifo.len();
            self.fifo.resize(len + 2 * capacity, 0.0);
            // SAFETY: `len` is inside the vector, which has room for `capacity` stereo frames
            // after it.
            let mut out = unsafe { self.fifo.as_mut_ptr().add(len) }.cast::<u8>();
            // SAFETY: as above; the inputs are valid as the caller promises.
            let written = unsafe {
                resampler
                    .swr
                    .convert(&mut out, capacity as i32, inputs, count)
            }
            .map_err(failed)?
            .max(0) as usize;
            self.fifo.truncate(len + 2 * written);
        }
        Ok(())
    }
}

/// One open reader of [`AudioSources`].
#[derive(Debug)]
struct Slot {
    media: MediaId,
    path: String,
    stream: u32,
    reader: AudioReader,
    /// The clip it last read for, or was primed for.
    clip: Option<ClipId>,
    /// Pre-seeked for `clip`, not read yet: other voices leave it alone.
    primed: bool,
    used_block: u64,
    last_used: Instant,
}

impl Slot {
    fn serves(&self, media: MediaId, path: &str, stream: u32, sample_rate: u32) -> bool {
        self.media == media
            && self.path == path
            && self.stream == stream
            && self.reader.sample_rate() == sample_rate
    }
}

/// The [`PcmProvider`] over FFmpeg [`AudioReader`]s (playback.md §6.4, §7): at most 16 readers,
/// closed after 5 s unused. A media that cannot be opened is silent and logged once; a decode
/// error silences the voice until its clip ends.
#[derive(Debug, Default)]
pub struct AudioSources {
    slots: Vec<Slot>,
    block: u64,
    failed_open: HashMap<MediaId, Instant>,
    logged: HashSet<MediaId>,
    /// Clips whose reader failed while decoding, with the block that last asked for them:
    /// silent until the next clip.
    dead: HashMap<ClipId, u64>,
    last_sweep: Option<Instant>,
}

impl AudioSources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open readers.
    pub fn reader_count(&self) -> usize {
        self.slots.len()
    }

    /// Seeks of every open reader so far (tests).
    pub fn seeks(&self) -> u64 {
        self.slots.iter().map(|s| s.reader.seeks()).sum()
    }

    /// Closes every reader (stream closed, project replaced).
    pub fn close_all(&mut self) {
        self.slots.clear();
        self.dead.clear();
    }

    /// Pre-seeks readers for the voices that start within `(from, from + window]` and that no
    /// reader will reach by decoding forward (jump cuts), so no seek happens when they start.
    pub fn prefetch(&mut self, project: &Project, from: Time, window: Time, sample_rate: u32) {
        let until = from + window;
        let active: HashSet<ClipId> = project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start <= from && from < c.end())
            .map(|c| c.id)
            .collect();
        for track in project.tracks.iter().filter(|t| !t.muted) {
            for (i, clip) in track.clips.iter().enumerate() {
                if clip.start <= from || clip.start > until || clip.audio.is_none() {
                    continue;
                }
                let ClipSource::Media { media, source_in } = clip.source else {
                    continue;
                };
                let Some(asset) = project.media_asset(media) else {
                    continue;
                };
                let Some(stream) = asset.info.audio.first().map(|a| a.stream_index) else {
                    continue;
                };
                let path = asset.path.absolute.as_str();
                let serves = |s: &Slot| s.serves(media, path, stream, sample_rate);
                let tolerance = sample_offset(1, sample_rate);
                if self.slots.iter().any(|s| {
                    serves(s)
                        && (s.reader.position() - source_in).flicks().abs() < tolerance.flicks()
                }) {
                    continue; // already primed
                }
                // A split point: the reader of the previous clip continues into this one.
                let previous = i.checked_sub(1).map(|p| &track.clips[p]);
                let continues = previous.is_some_and(|p| {
                    p.end() == clip.start
                        && p.source.media() == Some(media)
                        && p.source_end() == Some(source_in)
                        && self.slots.iter().any(|s| serves(s) && s.clip == Some(p.id))
                });
                if continues || self.failed_open.contains_key(&media) {
                    continue;
                }
                let idle = self.slots.iter().position(|s| {
                    serves(s) && !s.primed && s.clip.is_none_or(|c| !active.contains(&c))
                });
                let slot = match idle {
                    Some(slot) => Some(slot),
                    None => self.open_slot(asset, stream, sample_rate),
                };
                let Some(slot) = slot else { continue };
                let entry = &mut self.slots[slot];
                let primed = entry
                    .reader
                    .seek(source_in)
                    .and_then(|()| entry.reader.prime(1024));
                if let Err(err) = primed {
                    self.log_once(media, &err);
                    self.slots.swap_remove(slot);
                    continue;
                }
                let entry = &mut self.slots[slot];
                entry.clip = Some(clip.id);
                entry.primed = true;
                entry.last_used = Instant::now();
            }
        }
    }

    fn log_once(&mut self, media: MediaId, err: &MediaError) {
        if self.logged.insert(media) {
            tracing::warn!(%media, %err, "audio unavailable; the voice is silent");
        }
    }

    /// Opens a reader for `asset` (closing the least recently used idle one at the limit).
    fn open_slot(
        &mut self,
        asset: &app_core::MediaAsset,
        stream: u32,
        sample_rate: u32,
    ) -> Option<usize> {
        if let Some(at) = self.failed_open.get(&asset.id) {
            if at.elapsed() < RETRY_AFTER {
                return None;
            }
            self.failed_open.remove(&asset.id);
        }
        if self.slots.len() >= MAX_READERS {
            let block = self.block;
            let lru = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.used_block != block)
                .min_by_key(|(_, s)| s.last_used)
                .map(|(i, _)| i)?;
            self.slots.swap_remove(lru);
        }
        let path = Path::new(&asset.path.absolute);
        match AudioReader::open(path, stream, asset.info.origin, sample_rate) {
            Ok(reader) => {
                self.slots.push(Slot {
                    media: asset.id,
                    path: asset.path.absolute.clone(),
                    stream,
                    reader,
                    clip: None,
                    primed: false,
                    used_block: 0,
                    last_used: Instant::now(),
                });
                Some(self.slots.len() - 1)
            }
            Err(err) => {
                self.failed_open.insert(asset.id, Instant::now());
                self.log_once(asset.id, &err);
                None
            }
        }
    }

    /// The reader for `voice`, positioned at its source time: one that is already there (the
    /// same clip, a split point, a primed jump cut), one shortly before it (decodes forward), an
    /// idle one of the same media (seeks), or a new one.
    fn reader_for(&mut self, voice: &Voice<'_>, stream: u32) -> Result<Option<usize>, MediaError> {
        let block = self.block;
        let media = voice.media.id;
        let path = voice.media.path.absolute.as_str();
        let rate = voice.sample_rate;
        let t = voice.source_time;
        let tolerance = sample_offset(1, rate).flicks().max(1) / 2 + 1;
        let free = |s: &Slot| s.serves(media, path, stream, rate) && s.used_block != block;
        let lead = |s: &Slot| (t - s.reader.position()).flicks();
        let owned = |s: &Slot| !s.primed || s.clip == Some(voice.clip);

        // Exactly at the source time: continue, even if the reader was read in this block (a
        // split point inside the block: the previous clip ended at the sample before).
        if let Some(i) = self.slots.iter().position(|s| {
            s.serves(media, path, stream, rate) && owned(s) && lead(s).abs() < tolerance
        }) {
            return Ok(Some(i));
        }
        if let Some(i) = self
            .slots
            .iter()
            .position(|s| free(s) && owned(s) && lead(s) > 0 && lead(s) <= FORWARD_SKIP.flicks())
        {
            let frames = samples_before(Time(lead(&self.slots[i])), rate) as usize;
            self.slots[i].reader.discard(frames)?;
            return Ok(Some(i));
        }
        let idle = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| free(s) && owned(s))
            .min_by_key(|(_, s)| s.last_used)
            .map(|(i, _)| i);
        let slot = match idle {
            Some(i) => Some(i),
            None => self.open_slot(voice.media, stream, rate),
        };
        if let Some(i) = slot {
            let reader = &mut self.slots[i].reader;
            // A new reader starts at media time 0 without a seek.
            if (t - reader.position()).flicks().abs() >= tolerance {
                reader.seek(t)?;
            }
        }
        Ok(slot)
    }

    fn sweep(&mut self) {
        let now = Instant::now();
        if self
            .last_sweep
            .is_some_and(|at| now - at < Duration::from_secs(1))
        {
            return;
        }
        self.last_sweep = Some(now);
        self.slots.retain(|s| now - s.last_used < IDLE_CLOSE);
        // A dead clip is forgotten once its voice stopped asking.
        let block = self.block;
        self.dead.retain(|_, asked| block - *asked < 8);
    }
}

impl PcmProvider for AudioSources {
    fn begin_block(&mut self) {
        self.block += 1;
        self.sweep();
    }

    fn read(&mut self, voice: &Voice<'_>, out: &mut [f32]) -> bool {
        if let Some(asked) = self.dead.get_mut(&voice.clip) {
            *asked = self.block;
            return false;
        }
        let Some(stream) = voice.media.info.audio.first().map(|a| a.stream_index) else {
            return false;
        };
        let slot = match self.reader_for(voice, stream) {
            Ok(Some(slot)) => slot,
            Ok(None) => return false,
            Err(err) => {
                self.log_once(voice.media.id, &err);
                self.dead.insert(voice.clip, self.block);
                return false;
            }
        };
        let entry = &mut self.slots[slot];
        entry.clip = Some(voice.clip);
        entry.primed = false;
        entry.used_block = self.block;
        entry.last_used = Instant::now();
        match entry.reader.read(out) {
            Ok(()) => true,
            Err(err) => {
                self.slots.swap_remove(slot);
                self.log_once(voice.media.id, &err);
                self.dead.insert(voice.clip, self.block);
                false
            }
        }
    }
}
