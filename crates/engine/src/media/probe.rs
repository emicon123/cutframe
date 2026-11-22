//! Probing a media file: what it is, its streams, timing, rotation, pixel aspect, colour and HDR
//! metadata (design `docs/design/media-import.md` §3, §5).
//!
//! The probe opens a decoder for every stream it reports and decodes one frame from each, so a
//! file that "probes fine but cannot decode" fails at import (`UnsupportedCodec` / `Damaged`)
//! instead of on the timeline. It always decodes in software, so import works on any machine.
//!
//! This does file I/O and may take a while on slow disks or large files: run it in a worker job,
//! never inside an IPC handler (ADR-0004).

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};

use app_core::media::{
    AudioStreamInfo, ColorInfo, DolbyVisionInfo, HdrInfo, MediaInfo, MediaKind, Rotation,
    VideoStreamInfo, hdr_transfer,
};
use app_core::{Rational, Time};
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

use super::decode::{open_decoder, send_and_receive};
use super::error::{MediaError, MediaErrorKind};
use super::input::Input;

/// The probe result: the kind of media and everything the model keeps about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Probed {
    pub kind: MediaKind,
    pub info: MediaInfo,
}

/// Packets read from the start of the file while waiting for the first frame of every stream.
const MAX_PACKETS_FOR_FIRST_FRAMES: usize = 4096;

/// Probes `path`; see the module docs.
pub fn probe(path: &Path) -> Result<Probed, MediaError> {
    probe_cancellable(path, &AtomicBool::new(false))
}

/// [`probe`] that stops with [`MediaErrorKind::Cancelled`] soon after `cancel` is set, also
/// while FFmpeg is blocked on a slow disk, network share or cloud-only file.
pub fn probe_cancellable(path: &Path, cancel: &AtomicBool) -> Result<Probed, MediaError> {
    let mut input = Input::open(path, cancel)?;
    let container = input.format_name();
    let selection = select_streams(&input)?;
    let first = decode_first_frames(&mut input, &selection, cancel)?;

    let audio_streams: Vec<usize> = selection
        .audio
        .iter()
        .copied()
        .filter(|index| first.audio_ok.contains(index))
        .collect();
    let mut video = selection
        .video
        .map(|index| video_info(&input, index, first.video_frame.as_ref()));
    if let (Some(video), Some(grid)) = (video.as_mut(), selection.grid) {
        // A HEIF tile grid: the picture is the composed grid, rotated by the item's `irot`.
        (video.width, video.height) = (grid.width, grid.height);
        video.rotation = heif_rotation(path).unwrap_or_default();
    }
    if video.is_none() && audio_streams.is_empty() {
        return Err(MediaError::new(
            MediaErrorKind::Damaged,
            "no audio stream produced a frame",
        ));
    }

    let streams = input.streams();
    let is_image = video.is_some()
        && (selection.grid.is_some() || is_still_image(&container, streams, selection.video));
    let kind = match (&video, is_image) {
        (None, _) => MediaKind::Audio,
        (Some(_), true) => MediaKind::Image,
        (Some(_), false) => MediaKind::Video,
    };
    let audio: Vec<AudioStreamInfo> = if kind == MediaKind::Image {
        Vec::new()
    } else {
        audio_streams
            .iter()
            .filter_map(|&index| audio_info(streams[index], index))
            .collect()
    };

    let used: Vec<usize> = selection
        .video
        .into_iter()
        .chain(audio.iter().map(|a| a.stream_index as usize))
        .collect();
    let origin = origin(&input, &used);
    let duration = if kind == MediaKind::Image {
        None
    } else {
        let known = known_end(&input, &used);
        // Always read the tail: it finds the end when the container states none, and it catches
        // truncated files whose index points past the end of the data.
        let scanned = scan_tail(&mut input, &used, cancel, known.is_none())?;
        let Some(end) = known.or(scanned) else {
            return Err(MediaError::new(
                MediaErrorKind::Damaged,
                "the duration is unknown",
            ));
        };
        let duration = end - origin;
        if duration <= Time::ZERO {
            return Err(MediaError::new(
                MediaErrorKind::Damaged,
                "the duration is unknown or zero",
            ));
        }
        Some(duration)
    };

    if kind == MediaKind::Image
        && let Some(video) = video.as_mut()
    {
        video.frame_rate = None;
    }
    Ok(Probed {
        kind,
        info: MediaInfo {
            container,
            origin,
            duration,
            video,
            audio,
        },
    })
}

/// The `irot` rotation of a HEIF file's primary item, read from the head of the file.
fn heif_rotation(path: &Path) -> Option<Rotation> {
    use std::io::Read;
    let mut head = Vec::with_capacity(super::heif::HEAD_BYTES);
    std::fs::File::open(path)
        .ok()?
        .take(super::heif::HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    super::heif::primary_item_rotation(&head)
}

/// A HEIF tile grid (FFmpeg 7.1 stream group): its output size and its first tile stream.
#[derive(Debug, Clone, Copy)]
struct Grid {
    first_tile: usize,
    width: u32,
    height: u32,
}

fn tile_grid(input: &Input<'_>) -> Option<Grid> {
    let ctx = input.ctx();
    if ctx.stream_groups.is_null() {
        return None;
    }
    // SAFETY: `stream_groups` holds `nb_stream_groups` valid group pointers owned by the context.
    let groups = unsafe {
        std::slice::from_raw_parts(
            ctx.stream_groups.cast::<&ffi::AVStreamGroup>(),
            ctx.nb_stream_groups as usize,
        )
    };
    groups.iter().find_map(|group| {
        if group.type_ != ffi::AV_STREAM_GROUP_PARAMS_TILE_GRID || group.nb_streams == 0 {
            return None;
        }
        // SAFETY: for a tile-grid group `params.tile_grid` is the active union member, and
        // `streams` holds `nb_streams` valid stream pointers.
        let (grid, first) = unsafe { (&*group.params.tile_grid, &**group.streams) };
        Some(Grid {
            first_tile: usize::try_from(first.index).ok()?,
            width: u32::try_from(grid.width).ok()?,
            height: u32::try_from(grid.height).ok()?,
        })
    })
}

/// The streams the probe reports.
#[derive(Debug)]
struct Selection {
    video: Option<usize>,
    /// Set when the picture is a HEIF tile grid; `video` is then its first tile.
    grid: Option<Grid>,
    /// Decodable audio streams, default stream first.
    audio: Vec<usize>,
}

fn is_type(stream: &ffi::AVStream, media_type: ffi::AVMediaType) -> bool {
    // SAFETY: every stream of an opened context has codec parameters.
    unsafe { (*stream.codecpar).codec_type == media_type }
}

pub(crate) fn par(stream: &ffi::AVStream) -> &ffi::AVCodecParameters {
    // SAFETY: every stream of an opened context has codec parameters, owned by the stream.
    unsafe { &*stream.codecpar }
}

pub(crate) fn decoder_for(stream: &ffi::AVStream) -> Option<rsmpeg::avcodec::AVCodecRef<'static>> {
    AVCodec::find_decoder(par(stream).codec_id)
}

pub(crate) fn codec_name(id: ffi::AVCodecID) -> String {
    // SAFETY: returns a static NUL-terminated string for any id ("unknown_codec" if unknown).
    unsafe { CStr::from_ptr(ffi::avcodec_get_name(id)) }
        .to_string_lossy()
        .into_owned()
}

fn has_disposition(stream: &ffi::AVStream, flag: u32) -> bool {
    stream.disposition & flag as i32 != 0
}

/// Best video stream: not an attached picture (cover art), default disposition first, then the
/// largest picture. Audio: every stream with a decoder, default first, then in file order.
fn select_streams(input: &Input<'_>) -> Result<Selection, MediaError> {
    let streams = input.streams();
    let grid = tile_grid(input);
    let video = grid.map(|g| g.first_tile).or_else(|| {
        (0..streams.len())
            .filter(|&i| {
                is_type(streams[i], ffi::AVMEDIA_TYPE_VIDEO)
                    && !has_disposition(streams[i], ffi::AV_DISPOSITION_ATTACHED_PIC)
            })
            .max_by_key(|&i| {
                let p = par(streams[i]);
                (
                    has_disposition(streams[i], ffi::AV_DISPOSITION_DEFAULT),
                    i64::from(p.width) * i64::from(p.height),
                    std::cmp::Reverse(i),
                )
            })
    });
    let audio_all: Vec<usize> = (0..streams.len())
        .filter(|&i| is_type(streams[i], ffi::AVMEDIA_TYPE_AUDIO))
        .collect();
    let mut audio: Vec<usize> = audio_all
        .iter()
        .copied()
        .filter(|&i| decoder_for(streams[i]).is_some())
        .collect();
    audio.sort_by_key(|&i| !has_disposition(streams[i], ffi::AV_DISPOSITION_DEFAULT));

    if let Some(index) = video
        && decoder_for(streams[index]).is_none()
    {
        return Err(MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!(
                "no decoder for video codec {}",
                codec_name(par(streams[index]).codec_id)
            ),
        ));
    }
    if video.is_none() && audio.is_empty() {
        return Err(match audio_all.first() {
            Some(&index) => MediaError::new(
                MediaErrorKind::UnsupportedCodec,
                format!(
                    "no decoder for audio codec {}",
                    codec_name(par(streams[index]).codec_id)
                ),
            ),
            None => MediaError::new(
                MediaErrorKind::UnsupportedFormat,
                "no video or audio stream",
            ),
        });
    }
    Ok(Selection { video, grid, audio })
}

/// What decoding the first frames found.
#[derive(Debug, Default)]
struct FirstFrames {
    video_frame: Option<AVFrame>,
    /// Audio streams that produced a frame.
    audio_ok: Vec<usize>,
}

/// Decodes until every selected stream produced one frame (or the packet budget or the file
/// ends). A video stream that produces no frame makes the file `Damaged`; an audio stream that
/// produces none is dropped from the result.
fn decode_first_frames(
    input: &mut Input<'_>,
    selection: &Selection,
    cancel: &AtomicBool,
) -> Result<FirstFrames, MediaError> {
    let mut decoders: BTreeMap<usize, AVCodecContext> = BTreeMap::new();
    {
        let streams = input.streams();
        if let Some(index) = selection.video {
            decoders.insert(index, open_decoder(streams[index])?);
        }
        for &index in &selection.audio {
            match open_decoder(streams[index]) {
                Ok(decoder) => {
                    decoders.insert(index, decoder);
                }
                Err(err) => tracing::warn!(index, %err, "audio stream skipped"),
            }
        }
    }

    let mut first = FirstFrames::default();
    let mut last_error = None;
    let done = |index: usize, frame: AVFrame, first: &mut FirstFrames| {
        if Some(index) == selection.video {
            first.video_frame = Some(frame);
        } else {
            first.audio_ok.push(index);
        }
    };

    let mut packets = 0;
    while !decoders.is_empty() && packets < MAX_PACKETS_FOR_FIRST_FRAMES {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::new(MediaErrorKind::Cancelled, "cancelled"));
        }
        let packet = match input.read_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(err) if err.kind == MediaErrorKind::Cancelled => return Err(err),
            Err(err) => {
                last_error = Some(err);
                break;
            }
        };
        packets += 1;
        let index = packet.stream_index as usize;
        let Some(decoder) = decoders.get_mut(&index) else {
            continue;
        };
        match send_and_receive(decoder, Some(&packet)) {
            Ok(Some(frame)) => {
                decoders.remove(&index);
                done(index, frame, &mut first);
            }
            Ok(None) => {}
            Err(err) => last_error = Some(err),
        }
    }
    // Flush: decoders with frame delay (B-frames, threads) hold frames back until the end.
    for (index, mut decoder) in std::mem::take(&mut decoders) {
        if let Ok(Some(frame)) = send_and_receive(&mut decoder, None) {
            done(index, frame, &mut first);
        }
    }

    if selection.video.is_some() && first.video_frame.is_none() {
        let detail = last_error.map_or_else(
            || "the video stream produced no frame".to_owned(),
            |err| format!("the video stream produced no frame ({})", err.detail),
        );
        return Err(MediaError::new(MediaErrorKind::Damaged, detail));
    }
    Ok(first)
}

fn rational(r: ffi::AVRational) -> Option<Rational> {
    Rational::new(i64::from(r.num), i64::from(r.den)).ok()
}

/// `ts` in `time_base` as model time; `None` for `AV_NOPTS_VALUE` or an invalid time base.
pub(crate) fn ts_to_time(ts: i64, time_base: ffi::AVRational) -> Option<Time> {
    if ts == ffi::AV_NOPTS_VALUE {
        return None;
    }
    rational(time_base).map(|tb| Time::from_ticks(ts, tb))
}

const AV_TIME_BASE_Q: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: ffi::AV_TIME_BASE as i32,
};

/// Media time zero: the earliest start of the used streams, else the container start, else 0.
fn origin(input: &Input<'_>, used: &[usize]) -> Time {
    let streams = input.streams();
    used.iter()
        .filter_map(|&i| ts_to_time(streams[i].start_time, streams[i].time_base))
        .min()
        .or_else(|| ts_to_time(input.ctx().start_time, AV_TIME_BASE_Q))
        .unwrap_or(Time::ZERO)
}

/// The end of the media, in container time, from stream or container durations.
fn known_end(input: &Input<'_>, used: &[usize]) -> Option<Time> {
    let streams = input.streams();
    let stream_end = used
        .iter()
        .filter_map(|&i| {
            let s = streams[i];
            let start = ts_to_time(s.start_time, s.time_base).unwrap_or(Time::ZERO);
            ts_to_time(s.duration, s.time_base).map(|d| start + d)
        })
        .max();
    stream_end.or_else(|| {
        let ctx = input.ctx();
        let start = ts_to_time(ctx.start_time, AV_TIME_BASE_Q).unwrap_or(Time::ZERO);
        ts_to_time(ctx.duration, AV_TIME_BASE_Q).map(|d| start + d)
    })
}

/// Packets read after seeking to the end when the duration is already known.
const TAIL_CHECK_PACKETS: usize = 32;

/// Seeks near the end and reads packets. Returns the end of the last packet when `need_end`
/// (the container states no duration: browser-recorded WebM, some MKV; if seeking fails it reads
/// on to the end of the file). A read error in the tail means the file is truncated or damaged.
fn scan_tail(
    input: &mut Input<'_>,
    used: &[usize],
    cancel: &AtomicBool,
    need_end: bool,
) -> Result<Option<Time>, MediaError> {
    // SAFETY: an opened input context. Seeks to the last keyframe at or before the end.
    let ret =
        unsafe { ffi::avformat_seek_file(input.as_mut_ptr(), -1, i64::MIN, i64::MAX, i64::MAX, 0) };
    if ret < 0 {
        if !need_end {
            return Ok(None);
        }
        tracing::debug!("seek to the end failed; reading on to find the duration");
    }
    let time_bases: BTreeMap<usize, ffi::AVRational> = used
        .iter()
        .map(|&i| (i, input.streams()[i].time_base))
        .collect();
    let mut end: Option<Time> = None;
    let mut packets = 0;
    while need_end || packets < TAIL_CHECK_PACKETS {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::new(MediaErrorKind::Cancelled, "cancelled"));
        }
        let bad_tail = match input.read_packet() {
            Ok(Some(packet)) if packet.flags & ffi::AV_PKT_FLAG_CORRUPT as i32 == 0 => {
                packets += 1;
                Ok(packet)
            }
            // E.g. an MP4 whose index points past the end of the (truncated) data.
            Ok(Some(_)) => Err("the end of the file is corrupt or missing".to_owned()),
            Ok(None) => break,
            Err(err) if err.kind == MediaErrorKind::Cancelled => return Err(err),
            Err(err) => Err(format!(
                "the end of the file cannot be read: {}",
                err.detail
            )),
        };
        let packet = match bad_tail {
            Ok(packet) => packet,
            Err(detail) => return tail_error(need_end, end, detail),
        };
        let Some(&tb) = time_bases.get(&(packet.stream_index as usize)) else {
            continue;
        };
        let ts = if packet.pts != ffi::AV_NOPTS_VALUE {
            packet.pts
        } else {
            packet.dts
        };
        if let Some(start) = ts_to_time(ts, tb) {
            let length = ts_to_time(packet.duration.max(0), tb).unwrap_or(Time::ZERO);
            end = end.max(Some(start + length));
        }
    }
    Ok(end)
}

/// A bad tail. Where the container promised the data (stated duration, an MP4/MOV index), the
/// file is damaged. Where it states no duration (an OBS MKV whose recording crashed ends in a
/// partial cluster), the good packets before the damage still play: their end is the duration.
fn tail_error(
    need_end: bool,
    end: Option<Time>,
    detail: String,
) -> Result<Option<Time>, MediaError> {
    match end {
        Some(end) if need_end => {
            tracing::warn!(%detail, ?end, "file ends in damaged data; using the last good packet as its end");
            Ok(Some(end))
        }
        _ => Err(MediaError::new(MediaErrorKind::Damaged, detail)),
    }
}

fn side_data<'a>(
    data: *const ffi::AVPacketSideData,
    count: i32,
    kind: ffi::AVPacketSideDataType,
) -> Option<&'a [u8]> {
    if data.is_null() || count <= 0 {
        return None;
    }
    // SAFETY: `data` holds `count` entries (FFmpeg's contract for `coded_side_data`).
    let entry = unsafe { ffi::av_packet_side_data_get(data, count, kind) };
    if entry.is_null() {
        return None;
    }
    // SAFETY: a found entry points at `size` valid bytes owned by the codec parameters.
    let entry = unsafe { &*entry };
    if entry.data.is_null() {
        return None;
    }
    // SAFETY: as above.
    Some(unsafe { std::slice::from_raw_parts(entry.data, entry.size) })
}

fn frame_side_data(frame: &AVFrame, kind: ffi::AVFrameSideDataType) -> Option<&[u8]> {
    // SAFETY: `frame` is a decoded frame; the returned entry, if any, is owned by it.
    let entry = unsafe { ffi::av_frame_get_side_data(frame.as_ptr(), kind) };
    if entry.is_null() {
        return None;
    }
    // SAFETY: a found entry points at `size` valid bytes owned by the frame.
    let entry = unsafe { &*entry };
    if entry.data.is_null() {
        return None;
    }
    // SAFETY: as above.
    Some(unsafe { std::slice::from_raw_parts(entry.data, entry.size) })
}

/// Reads a `T` from side data that FFmpeg documents as holding one `T`.
fn read_struct<T: Copy>(bytes: &[u8]) -> Option<T> {
    // SAFETY: the length is checked; `read_unaligned` copes with any alignment, and `T` is a
    // plain C struct of integers for every caller.
    (bytes.len() >= size_of::<T>()).then(|| unsafe { ptr::read_unaligned(bytes.as_ptr().cast()) })
}

fn color_name(name: *const std::ffi::c_char) -> Option<String> {
    NonNull::new(name.cast_mut()).map(|name| {
        // SAFETY: FFmpeg returns static NUL-terminated names.
        unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

fn color_info(par: &ffi::AVCodecParameters, frame: Option<&AVFrame>) -> ColorInfo {
    let primaries = pick(
        par.color_primaries,
        frame.map(|f| f.color_primaries),
        ffi::AVCOL_PRI_UNSPECIFIED,
    );
    let transfer = pick(
        par.color_trc,
        frame.map(|f| f.color_trc),
        ffi::AVCOL_TRC_UNSPECIFIED,
    );
    let matrix = pick(
        par.color_space,
        frame.map(|f| f.colorspace),
        ffi::AVCOL_SPC_UNSPECIFIED,
    );
    let range = pick(
        par.color_range,
        frame.map(|f| f.color_range),
        ffi::AVCOL_RANGE_UNSPECIFIED,
    );
    // The `av_color_*_name` functions accept any value and return a static string or NULL.
    ColorInfo {
        // SAFETY: see above.
        primaries: primaries.and_then(|v| color_name(unsafe { ffi::av_color_primaries_name(v) })),
        // SAFETY: see above.
        transfer: transfer.and_then(|v| color_name(unsafe { ffi::av_color_transfer_name(v) })),
        // SAFETY: see above.
        matrix: matrix.and_then(|v| color_name(unsafe { ffi::av_color_space_name(v) })),
        // SAFETY: see above.
        range: range.and_then(|v| color_name(unsafe { ffi::av_color_range_name(v) })),
    }
}

/// The container's value, else the decoded frame's, unless both are unspecified.
fn pick<T: PartialEq + Copy>(container: T, frame: Option<T>, unspecified: T) -> Option<T> {
    [Some(container), frame]
        .into_iter()
        .flatten()
        .find(|v| *v != unspecified)
}

fn pixel_format_info(format: i32) -> (String, u8) {
    // SAFETY: returns NULL for unknown formats, else a static descriptor.
    let desc = unsafe { ffi::av_pix_fmt_desc_get(format) };
    if desc.is_null() {
        return ("none".to_owned(), 8);
    }
    // SAFETY: a static descriptor with a NUL-terminated name.
    let desc = unsafe { &*desc };
    // SAFETY: as above.
    let name = unsafe { CStr::from_ptr(desc.name) }
        .to_string_lossy()
        .into_owned();
    let depth = u8::try_from(desc.comp[0].depth).unwrap_or(8);
    (name, depth)
}

/// Rotation from the display matrix; mirroring is ignored and logged.
fn display_rotation(par: &ffi::AVCodecParameters) -> Option<Rotation> {
    let bytes = side_data(
        par.coded_side_data,
        par.nb_coded_side_data,
        ffi::AV_PKT_DATA_DISPLAYMATRIX,
    )?;
    let matrix: [i32; 9] = read_struct(bytes)?;
    // SAFETY: `matrix` is a 3x3 display matrix as FFmpeg defines it.
    let ccw = unsafe { ffi::av_display_rotation_get(matrix.as_ptr()) };
    let determinant =
        i64::from(matrix[0]) * i64::from(matrix[4]) - i64::from(matrix[1]) * i64::from(matrix[3]);
    if determinant < 0 {
        tracing::info!("display matrix mirrors the picture; mirroring is ignored");
    }
    Some(Rotation::from_degrees_cw(-ccw))
}

/// EXIF orientation, as the JPEG decoder exports it in the frame metadata (FFmpeg 7.1).
fn exif_rotation(frame: &AVFrame) -> Option<Rotation> {
    // SAFETY: the frame's metadata dictionary, may be NULL; the key is NUL-terminated.
    let entry =
        unsafe { ffi::av_dict_get(frame.metadata, c"Orientation".as_ptr(), ptr::null(), 0) };
    if entry.is_null() {
        return None;
    }
    // SAFETY: a found entry has a NUL-terminated value.
    let value = unsafe { CStr::from_ptr((*entry).value) }.to_string_lossy();
    let rotation = match value.trim() {
        "3" | "4" => Rotation::Cw180,
        "6" | "7" => Rotation::Cw90,
        "8" | "5" => Rotation::Cw270,
        _ => Rotation::None,
    };
    if matches!(value.trim(), "2" | "4" | "5" | "7") {
        tracing::info!(orientation = %value, "EXIF orientation mirrors the picture; mirroring is ignored");
    }
    Some(rotation)
}

fn hdr_info(
    par: &ffi::AVCodecParameters,
    transfer: Option<&str>,
    frame: Option<&AVFrame>,
) -> Option<HdrInfo> {
    let (data, count) = (par.coded_side_data, par.nb_coded_side_data);
    let dolby_vision = side_data(data, count, ffi::AV_PKT_DATA_DOVI_CONF)
        .and_then(read_struct::<ffi::AVDOVIDecoderConfigurationRecord>)
        .map(|dv| DolbyVisionInfo {
            profile: dv.dv_profile,
            level: dv.dv_level,
            compatibility_id: dv.dv_bl_signal_compatibility_id,
        });
    let transfer = hdr_transfer(transfer, dolby_vision)?;

    let mastering = side_data(data, count, ffi::AV_PKT_DATA_MASTERING_DISPLAY_METADATA)
        .or_else(|| {
            frame.and_then(|f| frame_side_data(f, ffi::AV_FRAME_DATA_MASTERING_DISPLAY_METADATA))
        })
        .and_then(read_struct::<ffi::AVMasteringDisplayMetadata>)
        .filter(|m| m.has_luminance != 0 && m.max_luminance.den > 0 && m.max_luminance.num > 0)
        .map(|m| {
            let nits = (i64::from(m.max_luminance.num) + i64::from(m.max_luminance.den) / 2)
                / i64::from(m.max_luminance.den);
            u32::try_from(nits).unwrap_or(u32::MAX)
        });
    let max_cll = side_data(data, count, ffi::AV_PKT_DATA_CONTENT_LIGHT_LEVEL)
        .or_else(|| frame.and_then(|f| frame_side_data(f, ffi::AV_FRAME_DATA_CONTENT_LIGHT_LEVEL)))
        .and_then(read_struct::<ffi::AVContentLightMetadata>)
        .map(|c| c.MaxCLL)
        .filter(|&nits| nits > 0);
    Some(HdrInfo {
        transfer,
        mastering_max_nits: mastering,
        max_cll_nits: max_cll,
        dolby_vision,
    })
}

fn video_info(input: &Input<'_>, index: usize, frame: Option<&AVFrame>) -> VideoStreamInfo {
    let stream = input.streams()[index];
    let par = par(stream);
    let (width, height) = match frame {
        Some(f) if f.width > 0 && f.height > 0 && (par.width <= 0 || par.height <= 0) => {
            (f.width, f.height)
        }
        _ => (par.width, par.height),
    };
    let sample_aspect = rational(stream.sample_aspect_ratio)
        .or_else(|| rational(par.sample_aspect_ratio))
        .or_else(|| frame.and_then(|f| rational(f.sample_aspect_ratio)))
        .unwrap_or(Rational::ONE);
    let rotation = display_rotation(par)
        .or_else(|| frame.and_then(exif_rotation))
        .unwrap_or_default();
    let frame_rate = rational(stream.avg_frame_rate)
        .or_else(|| rational(stream.r_frame_rate))
        .or_else(|| rational(par.framerate))
        .or_else(|| {
            tracing::debug!(index, "no frame rate stated; assuming 30 fps");
            Rational::new(30, 1).ok()
        });
    let format = frame.map_or(par.format, |f| f.format);
    let (pixel_format, bit_depth) = pixel_format_info(format);
    let color = color_info(par, frame);
    let hdr = hdr_info(par, color.transfer.as_deref(), frame);
    VideoStreamInfo {
        stream_index: index as u32,
        codec: codec_name(par.codec_id),
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        sample_aspect,
        rotation,
        frame_rate,
        pixel_format,
        bit_depth,
        color,
        hdr,
    }
}

fn audio_info(stream: &ffi::AVStream, index: usize) -> Option<AudioStreamInfo> {
    let par = par(stream);
    let sample_rate = u32::try_from(par.sample_rate).ok().filter(|&r| r > 0)?;
    let channels = u16::try_from(par.ch_layout.nb_channels)
        .ok()
        .filter(|&c| c > 0)?;
    Some(AudioStreamInfo {
        stream_index: index as u32,
        codec: codec_name(par.codec_id),
        sample_rate,
        channels,
    })
}

/// Stills come from FFmpeg's image demuxers (`image2`, `png_pipe`, `webp_pipe`, ...) or are a
/// one-frame video stream. Animated GIF (`gif` demuxer) and APNG are video; animated WebP decodes
/// only its first frame in FFmpeg 7.1 and is a still.
fn is_still_image(container: &str, streams: &[&ffi::AVStream], video: Option<usize>) -> bool {
    if container == "image2" || container.ends_with("_pipe") {
        return true;
    }
    video.is_some_and(|i| streams[i].nb_frames == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ffmpeg_timestamps_and_skips_missing_ones() {
        let tb = |num, den| ffi::AVRational { num, den };
        assert_eq!(
            ts_to_time(12_800, tb(1, 12_800)),
            Some(Time::from_seconds(1))
        );
        assert_eq!(
            ts_to_time(1_000_000, AV_TIME_BASE_Q),
            Some(Time::from_seconds(1))
        );
        assert_eq!(ts_to_time(ffi::AV_NOPTS_VALUE, tb(1, 1000)), None);
        assert_eq!(ts_to_time(1, tb(1, 0)), None);
        assert_eq!(ts_to_time(1, tb(0, 1)), None);
    }

    #[test]
    fn a_bad_tail_keeps_the_good_part_only_without_a_stated_duration() {
        let end = Some(Time::from_seconds(3));
        // No stated duration (crashed MKV recording): the last good packet is the end.
        assert_eq!(tail_error(true, end, "partial cluster".into()), Ok(end));
        // A stated duration or an index (MP4/MOV) promised data that is missing.
        let err = tail_error(false, end, "partial file".into()).unwrap_err();
        assert_eq!(err.kind, MediaErrorKind::Damaged);
        // Nothing good was read.
        let err = tail_error(true, None, "garbage".into()).unwrap_err();
        assert_eq!(err.kind, MediaErrorKind::Damaged);
    }

    #[test]
    fn container_values_win_over_frame_values() {
        assert_eq!(pick(1, Some(2), 0), Some(1));
        assert_eq!(pick(0, Some(2), 0), Some(2));
        assert_eq!(pick(0, Some(0), 0), None);
        assert_eq!(pick(0, None, 0), None);
    }

    #[test]
    fn pixel_formats_report_their_bit_depth() {
        assert_eq!(
            pixel_format_info(ffi::AV_PIX_FMT_YUV420P),
            ("yuv420p".to_owned(), 8)
        );
        assert_eq!(
            pixel_format_info(ffi::AV_PIX_FMT_YUV420P10LE),
            ("yuv420p10le".to_owned(), 10)
        );
        assert_eq!(pixel_format_info(ffi::AV_PIX_FMT_P010LE).1, 10);
        assert_eq!(pixel_format_info(-1).0, "none");
    }
}
