//! Poster, filmstrip and waveform generators for the media panel (design §4, §5).
//!
//! All decode in software in the calling (job worker) thread, open their own FFmpeg contexts and
//! stop soon after `cancel` is set. Pictures come out upright: rotation (display matrix, EXIF,
//! HEIF `irot`) and pixel aspect are applied, HDR sources are tone-mapped.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use app_core::Time;
use app_core::media::{
    FILMSTRIP_FRAME_HEIGHT, FilmstripLayout, MediaAsset, MediaKind, POSTER_LONG_SIDE,
    VideoStreamInfo,
};

use super::error::{MediaError, MediaErrorKind};
use super::picture::{RgbImage, encode_jpeg, fit_long_side, frame_to_rgb};
use super::reader::VideoReader;
use super::still::decode_tile_grid;
use super::waveform::{Waveform, waveform};

fn no_product(what: &str) -> MediaError {
    MediaError::new(MediaErrorKind::UnsupportedFormat, what)
}

fn video_of(asset: &MediaAsset) -> Result<&VideoStreamInfo, MediaError> {
    asset
        .info
        .video
        .as_ref()
        .ok_or_else(|| no_product("no video stream"))
}

/// Size before rotation that becomes `upright` after `video`'s rotation.
fn stored_size(video: &VideoStreamInfo, upright: (u32, u32)) -> (u32, u32) {
    if video.rotation.swaps_axes() {
        (upright.1, upright.0)
    } else {
        upright
    }
}

/// Media time of the poster frame: `min(1 s, duration / 2)`; 0 for images.
pub fn poster_time(asset: &MediaAsset) -> Time {
    match asset.info.duration {
        Some(duration) if asset.kind == MediaKind::Video => {
            Time::from_seconds(1).min(Time::from_flicks(duration.flicks() / 2))
        }
        _ => Time::ZERO,
    }
}

/// The upright poster picture of a video or image: long side [`POSTER_LONG_SIDE`].
pub fn poster_image(
    path: &Path,
    asset: &MediaAsset,
    cancel: &AtomicBool,
) -> Result<RgbImage, MediaError> {
    if !asset.has_poster() {
        return Err(no_product("audio has no poster"));
    }
    let video = video_of(asset)?;
    let (width, height) = video.display_size();
    let upright = fit_long_side(width, height, POSTER_LONG_SIDE);
    let (w, h) = stored_size(video, upright);
    if asset.kind == MediaKind::Image
        && let Some(grid) = decode_tile_grid(path, cancel)?
    {
        return Ok(grid.resized(w, h)?.rotated(video.rotation));
    }
    let mut reader = VideoReader::open(path, video.stream_index, asset.info.origin, cancel)?;
    let t = poster_time(asset);
    let (frame, _) = if t > Time::ZERO {
        reader.seek_frame_at(t, None)?
    } else {
        reader.frame_at(t, None)?
    };
    Ok(frame_to_rgb(&frame, w, h, video)?.rotated(video.rotation))
}

/// The poster as JPEG.
pub fn poster(path: &Path, asset: &MediaAsset, cancel: &AtomicBool) -> Result<Vec<u8>, MediaError> {
    encode_jpeg(&poster_image(path, asset, cancel)?)
}

/// The smallest `w x h` with the aspect of `source` that covers `target`.
fn cover_size(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (u64::from(source.0.max(1)), u64::from(source.1.max(1)));
    let (tw, th) = (u64::from(target.0), u64::from(target.1));
    if sw * th >= tw * sh {
        ((sw * th).div_ceil(sh).max(tw) as u32, th as u32)
    } else {
        (tw as u32, (sh * tw).div_ceil(sw).max(th) as u32)
    }
}

/// Tile `tile` of a video's filmstrip: its frames side by side, each `frame_width x 96`, upright
/// and centre-cropped to the frame shape.
pub fn filmstrip_tile_image(
    path: &Path,
    asset: &MediaAsset,
    tile: u32,
    cancel: &AtomicBool,
) -> Result<RgbImage, MediaError> {
    let layout = FilmstripLayout::of(asset).ok_or_else(|| no_product("not a video"))?;
    let frames = layout
        .tile_frames(tile)
        .ok_or_else(|| no_product("tile out of range"))?;
    let video = video_of(asset)?;
    let frame_size = (layout.frame_width, FILMSTRIP_FRAME_HEIGHT);
    let upright = cover_size(video.display_size(), frame_size);
    let (w, h) = stored_size(video, upright);
    let mut canvas = RgbImage::filled(
        layout.frame_width * frames.len() as u32,
        frame_size.1,
        [0; 3],
    );
    let mut reader = VideoReader::open(path, video.stream_index, asset.info.origin, cancel)?;
    // Long-GOP sources (screen recordings) decode forward instead of seeking for every frame.
    let mut forward = false;
    for (slot, frame_index) in frames.enumerate() {
        let t = layout.frame_time(frame_index);
        let (frame, decoded_forward) = if forward {
            reader.frame_at(t, None)?
        } else {
            reader.seek_frame_at(t, Some(Time::from_flicks(layout.interval.flicks() / 2)))?
        };
        forward |= decoded_forward;
        let picture = frame_to_rgb(&frame, w, h, video)?.rotated(video.rotation);
        let picture = picture.cropped(
            (picture.width - frame_size.0) / 2,
            (picture.height - frame_size.1) / 2,
            frame_size.0,
            frame_size.1,
        );
        canvas.blit(&picture, i64::from(layout.frame_width) * slot as i64, 0);
    }
    Ok(canvas)
}

/// The filmstrip tile as JPEG.
pub fn filmstrip_tile(
    path: &Path,
    asset: &MediaAsset,
    tile: u32,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, MediaError> {
    encode_jpeg(&filmstrip_tile_image(path, asset, tile, cancel)?)
}

/// The waveform of a video with audio or an audio file.
pub fn asset_waveform(
    path: &Path,
    asset: &MediaAsset,
    cancel: &AtomicBool,
) -> Result<Waveform, MediaError> {
    if !asset.has_waveform() {
        return Err(no_product("no audio"));
    }
    waveform(path, &asset.info, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_size_keeps_the_aspect() {
        assert_eq!(cover_size((1920, 1080), (171, 96)), (171, 97));
        assert_eq!(cover_size((1080, 1920), (54, 96)), (54, 96));
        assert_eq!(cover_size((64, 64), (96, 96)), (96, 96));
        assert_eq!(cover_size((3840, 1080), (171, 96)), (342, 96));
    }

    #[test]
    fn poster_time_is_one_second_or_half_the_duration() {
        let mut asset = app_core::media::samples::video(1);
        assert_eq!(poster_time(&asset), Time::from_seconds(1));
        asset.info.duration = Some(Time::from_seconds(1));
        assert_eq!(
            poster_time(&asset),
            Time::from_flicks(Time::from_seconds(1).flicks() / 2)
        );
        assert_eq!(poster_time(&app_core::media::samples::image(2)), Time::ZERO);
    }
}
