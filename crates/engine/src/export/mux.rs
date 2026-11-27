//! The MP4 muxer of export (export.md §4.6): `libavformat`'s `mp4` muxer on the partial file,
//! `movflags=+faststart` (the trailer moves `moov` before `mdat`), BT.709 / limited-range colour
//! tags on the video stream, `hvc1` for HEVC, `creation_time` = now, packets rescaled from the
//! encoder time bases (video `1 / fps`, CFR, `pts = n`; audio `1 / sample_rate`) and written with
//! `av_interleaved_write_frame`.
//!
//! [`StreamError`] is the error of the muxer and of the audio encoder ([`super::audio`]); the run
//! loop maps it to its error kinds (`DiskFull`, `WriteFailed`, `EncoderFailed`, `Internal`).
//!
//! Threading: a [`Muxer`] lives on the export thread; FFmpeg contexts are not shared.

use std::ffi::CString;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use rsmpeg::UnsafeDerefMut;
use rsmpeg::avcodec::{AVCodecContext, AVPacket};
use rsmpeg::avformat::{AVFormatContextOutput, AVIOContextContainer, AVIOContextCustom};
use rsmpeg::avutil::{AVDictionary, AVFrame, AVMem};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

/// Why writing the output (or encoding into it) failed. `String`s are English detail for the
/// log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// The output volume is full (`ENOSPC`).
    #[error("the output volume is full: {0}")]
    DiskFull(String),
    /// Any other output error: the file cannot be created or written.
    #[error("writing the output failed: {0}")]
    WriteFailed(String),
    /// An encoder failed to open, take a frame or return a packet.
    #[error("the encoder failed: {0}")]
    Encoder(String),
    /// Invalid parameters or an FFmpeg failure that is not about the file (a bug).
    #[error("internal muxing error: {0}")]
    Internal(String),
}

impl StreamError {
    /// An output (I/O) error from FFmpeg call `what`.
    fn write(err: &RsmpegError, what: &str) -> Self {
        let detail = format!("{what}: {err}");
        match err.raw_error() {
            Some(code) if code == ffi::AVERROR(ffi::ENOSPC) => Self::DiskFull(detail),
            _ => Self::WriteFailed(detail),
        }
    }
}

/// BT.709 limited range, left chroma siting: the colour of every exported video stream.
/// [`apply_color_tags`] puts it on the encoder context (so encoders write it into the bitstream
/// VUI) and [`Muxer::create`] on the stream (the MP4 `colr` nclx box).
pub const COLOR_PRIMARIES: ffi::AVColorPrimaries = ffi::AVCOL_PRI_BT709;
pub const COLOR_TRC: ffi::AVColorTransferCharacteristic = ffi::AVCOL_TRC_BT709;
pub const COLOR_SPACE: ffi::AVColorSpace = ffi::AVCOL_SPC_BT709;
pub const COLOR_RANGE: ffi::AVColorRange = ffi::AVCOL_RANGE_MPEG;
pub const CHROMA_LOCATION: ffi::AVChromaLocation = ffi::AVCHROMA_LOC_LEFT;

/// Sets the export colour tags (BT.709 primaries, transfer and matrix, limited range, left
/// chroma siting) on a video encoder context. Call before `avcodec_open2`.
pub fn apply_color_tags(context: &mut AVCodecContext) {
    // SAFETY: plain field writes on a valid, exclusively borrowed codec context.
    let raw = unsafe { context.deref_mut() };
    raw.color_primaries = COLOR_PRIMARIES;
    raw.color_trc = COLOR_TRC;
    raw.colorspace = COLOR_SPACE;
    raw.color_range = COLOR_RANGE;
    raw.chroma_sample_location = CHROMA_LOCATION;
}

/// The same tags on a frame sent to the encoder (some encoders read them from the frame).
pub fn tag_frame(frame: &mut AVFrame) {
    // SAFETY: plain field writes on a valid, exclusively borrowed frame.
    let raw = unsafe { frame.deref_mut() };
    raw.color_primaries = COLOR_PRIMARIES;
    raw.color_trc = COLOR_TRC;
    raw.colorspace = COLOR_SPACE;
    raw.color_range = COLOR_RANGE;
    raw.chroma_location = CHROMA_LOCATION;
}

/// How [`Muxer::create_with`] writes the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxOptions {
    /// `movflags=+faststart`: the trailer moves `moov` before `mdat` (always on in exports).
    pub faststart: bool,
    /// Test fault injection (export.md §11 `diskFull`): write through a custom I/O context that
    /// fails with `ENOSPC` once this many bytes were written. Needs `faststart: false` (the
    /// faststart rewrite reopens the file by name).
    pub write_limit: Option<u64>,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            faststart: true,
            write_limit: None,
        }
    }
}

/// `MKTAG('h', 'v', 'c', '1')`.
const HVC1: u32 = u32::from_le_bytes(*b"hvc1");

/// One stream: its index and the time bases packets are rescaled between.
#[derive(Debug, Clone, Copy)]
struct Track {
    index: i32,
    /// The encoder's time base (packets arrive in it).
    encoder: ffi::AVRational,
    /// The stream's time base after `avformat_write_header` (the muxer may change it).
    stream: ffi::AVRational,
    packets: u64,
}

/// An MP4 file being written: one video and one audio stream. Created on the partial path by
/// the run loop after both encoders are open; [`Self::finish`] writes the trailer and closes it.
/// Dropping it without `finish` closes an unfinished file (the run loop deletes it).
pub struct Muxer {
    output: AVFormatContextOutput,
    path: PathBuf,
    video: Track,
    audio: Track,
}

impl std::fmt::Debug for Muxer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Muxer")
            .field("path", &self.path)
            .field("video", &self.video)
            .field("audio", &self.audio)
            .finish_non_exhaustive()
    }
}

impl Muxer {
    /// Creates `path` (the partial file; an existing file is replaced) as MP4 with streams for
    /// the opened encoders `video` and `audio` and writes the header. Both encoders must have
    /// been opened with `AV_CODEC_FLAG_GLOBAL_HEADER`; `video` needs `time_base = 1 / fps` and
    /// `framerate = fps`.
    pub fn create(
        path: &Path,
        video: &AVCodecContext,
        audio: &AVCodecContext,
    ) -> Result<Self, StreamError> {
        Self::create_with(path, video, audio, MuxOptions::default())
    }

    /// [`Self::create`] with explicit [`MuxOptions`].
    pub fn create_with(
        path: &Path,
        video: &AVCodecContext,
        audio: &AVCodecContext,
        options: MuxOptions,
    ) -> Result<Self, StreamError> {
        let name = path
            .to_str()
            .and_then(|p| CString::new(p).ok())
            .ok_or_else(|| {
                StreamError::WriteFailed(format!("path {} is not valid UTF-8", path.display()))
            })?;
        let io_context = match options.write_limit {
            Some(limit) => Some(limited_file_io(path, limit)?),
            None => None,
        };
        let mut output = AVFormatContextOutput::builder()
            .format_name(c"mp4")
            .filename(&name)
            .maybe_io_context(io_context)
            .build()
            .map_err(|err| StreamError::write(&err, "create the output file"))?;

        let video_track = add_stream(&mut output, video, true)?;
        let audio_track = add_stream(&mut output, audio, false)?;
        {
            // SAFETY: `metadata` of a valid, exclusively borrowed output context; `av_dict_set`
            // allocates the dictionary when it is null and the context frees it.
            let raw = unsafe { output.deref_mut() };
            // SAFETY: see above; both strings are NUL-terminated literals.
            let ret = unsafe {
                ffi::av_dict_set(
                    &mut raw.metadata,
                    c"creation_time".as_ptr(),
                    c"now".as_ptr(),
                    0,
                )
            };
            if ret < 0 {
                return Err(StreamError::Internal("set creation_time".into()));
            }
        }

        let mut header_options = options
            .faststart
            .then(|| AVDictionary::new(c"movflags", c"+faststart", 0));
        output
            .write_header(&mut header_options)
            .map_err(|err| StreamError::write(&err, "write the MP4 header"))?;
        let stream_tb = |track: &Track| output.streams()[track.index as usize].time_base;
        let video = Track {
            stream: stream_tb(&video_track),
            ..video_track
        };
        let audio = Track {
            stream: stream_tb(&audio_track),
            ..audio_track
        };
        Ok(Self {
            output,
            path: path.to_path_buf(),
            video,
            audio,
        })
    }

    /// Video packets written so far. Encoder fallback is allowed only while this is 0
    /// (export.md §4.4).
    pub fn video_packets(&self) -> u64 {
        self.video.packets
    }

    /// Audio packets written so far.
    pub fn audio_packets(&self) -> u64 {
        self.audio.packets
    }

    /// Writes a packet from the video encoder (timestamps in its time base; CFR, so a packet
    /// without a duration lasts one frame).
    pub fn write_video(&mut self, mut packet: AVPacket) -> Result<(), StreamError> {
        if packet.duration <= 0 {
            packet.set_duration(1);
        }
        let track = self.video;
        self.write(&mut packet, track)?;
        self.video.packets += 1;
        Ok(())
    }

    /// Writes a packet from the audio encoder (timestamps in its time base, `1 / sample_rate`).
    pub fn write_audio(&mut self, mut packet: AVPacket) -> Result<(), StreamError> {
        let track = self.audio;
        self.write(&mut packet, track)?;
        self.audio.packets += 1;
        Ok(())
    }

    fn write(&mut self, packet: &mut AVPacket, track: Track) -> Result<(), StreamError> {
        packet.set_stream_index(track.index);
        packet.rescale_ts(track.encoder, track.stream);
        packet.set_pos(-1);
        self.output
            .interleaved_write_frame(packet)
            .map_err(|err| StreamError::write(&err, "write a packet"))
    }

    /// Flushes the interleaving queue and writes the trailer: with `+faststart` the muxer reads
    /// the file once more and rewrites it with `moov` first. Then closes the file, so the run
    /// loop can rename it. Call after both encoders were flushed into this muxer.
    pub fn finish(mut self) -> Result<PathBuf, StreamError> {
        self.output
            .write_trailer()
            .map_err(|err| StreamError::write(&err, "write the MP4 trailer"))?;
        let pb = self.output.pb;
        if !pb.is_null() {
            // SAFETY: `pb` is the output's open I/O context (set by the builder, freed only when
            // `self.output` drops below); `avio_flush` and the `error` read are valid on it.
            let error = unsafe {
                ffi::avio_flush(pb);
                (*pb).error
            };
            if error < 0 {
                return Err(StreamError::write(
                    &RsmpegError::AVError(error),
                    "flush the output file",
                ));
            }
        }
        let path = std::mem::take(&mut self.path);
        // Closes the file.
        drop(self);
        Ok(path)
    }
}

/// A write-only I/O context on a new file at `path` that fails every write with `ENOSPC` once
/// `limit` bytes were written (the `diskFull` test of export.md §11).
fn limited_file_io(path: &Path, limit: u64) -> Result<AVIOContextContainer, StreamError> {
    /// FFmpeg's `SEEK_*` and `AVSEEK_SIZE` values for the seek callback.
    const SEEK_SET: i32 = 0;
    const SEEK_CUR: i32 = 1;
    const SEEK_END: i32 = 2;
    const AVSEEK_SIZE: i32 = 0x10000;

    let file = std::fs::File::create(path)
        .map_err(|err| StreamError::WriteFailed(format!("create {}: {err}", path.display())))?;
    // (file, bytes written so far); shared by the write and seek callbacks.
    let shared = Arc::new(Mutex::new((file, 0u64)));
    let writer = Arc::clone(&shared);
    let write: rsmpeg::avformat::WritePacketCallback = Box::new(move |_, data: &[u8]| {
        let mut guard = writer.lock().unwrap_or_else(PoisonError::into_inner);
        let (file, written) = &mut *guard;
        if *written + data.len() as u64 > limit {
            return ffi::AVERROR(ffi::ENOSPC);
        }
        match file.write_all(data) {
            Ok(()) => {
                *written += data.len() as u64;
                i32::try_from(data.len()).unwrap_or(i32::MAX)
            }
            Err(_) => ffi::AVERROR(ffi::EIO),
        }
    });
    let seek: rsmpeg::avformat::SeekCallback = Box::new(move |_, offset: i64, whence: i32| {
        let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let file = &mut guard.0;
        if whence & AVSEEK_SIZE != 0 {
            return file
                .metadata()
                .map_or(i64::from(ffi::AVERROR(ffi::EIO)), |m| m.len() as i64);
        }
        let target = match whence & !ffi::AVSEEK_FORCE as i32 {
            SEEK_SET => SeekFrom::Start(offset.max(0) as u64),
            SEEK_CUR => SeekFrom::Current(offset),
            SEEK_END => SeekFrom::End(offset),
            _ => return i64::from(ffi::AVERROR(ffi::EINVAL)),
        };
        file.seek(target)
            .map_or(i64::from(ffi::AVERROR(ffi::EIO)), |pos| pos as i64)
    });
    let io = AVIOContextCustom::alloc_context(
        AVMem::new(4096),
        true,
        Vec::new(),
        None,
        Some(write),
        Some(seek),
    );
    Ok(AVIOContextContainer::Custom(io))
}

/// Adds a stream with the parameters of the opened encoder `context`.
fn add_stream(
    output: &mut AVFormatContextOutput,
    context: &AVCodecContext,
    video: bool,
) -> Result<Track, StreamError> {
    let time_base = context.time_base;
    if time_base.num <= 0 || time_base.den <= 0 {
        return Err(StreamError::Internal(format!(
            "encoder time base {}/{} is not set",
            time_base.num, time_base.den
        )));
    }
    let mut parameters = context.extract_codecpar();
    if video {
        // SAFETY: plain field writes on valid, owned codec parameters.
        let raw = unsafe { parameters.deref_mut() };
        raw.color_primaries = COLOR_PRIMARIES;
        raw.color_trc = COLOR_TRC;
        raw.color_space = COLOR_SPACE;
        raw.color_range = COLOR_RANGE;
        raw.chroma_location = CHROMA_LOCATION;
        // QuickTime, iOS and some TVs refuse `hev1` (parameter sets in-band).
        raw.codec_tag = if raw.codec_id == ffi::AV_CODEC_ID_HEVC {
            HVC1
        } else {
            0
        };
    }
    let mut stream = output.new_stream();
    stream.set_codecpar(parameters);
    stream.set_time_base(time_base);
    if video {
        let rate = context.framerate;
        stream.set_avg_frame_rate(rate);
        // SAFETY: plain field write on the valid stream just created.
        unsafe { UnsafeDerefMut::deref_mut(&mut *stream).r_frame_rate = rate };
    }
    Ok(Track {
        index: stream.index,
        encoder: time_base,
        stream: time_base,
        packets: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enospc_is_disk_full_and_other_errors_are_write_failures() {
        let full = RsmpegError::AVError(ffi::AVERROR(ffi::ENOSPC));
        assert!(matches!(
            StreamError::write(&full, "x"),
            StreamError::DiskFull(_)
        ));
        let denied = RsmpegError::AVError(ffi::AVERROR(ffi::EACCES));
        assert!(matches!(
            StreamError::write(&denied, "x"),
            StreamError::WriteFailed(_)
        ));
    }

    #[test]
    fn hvc1_is_the_quicktime_fourcc() {
        // MKTAG(a, b, c, d) = a | b << 8 | c << 16 | d << 24.
        let mktag = |t: &[u8; 4]| {
            u32::from(t[0]) | u32::from(t[1]) << 8 | u32::from(t[2]) << 16 | u32::from(t[3]) << 24
        };
        assert_eq!(HVC1, mktag(b"hvc1"));
    }
}
