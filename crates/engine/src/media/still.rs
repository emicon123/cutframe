//! Still pictures as one upright RGB image, including HEIF/HEIC tile grids.
//!
//! Phone HEIC photos are grids of HEVC tiles (often 512x512), which FFmpeg 7.1 exposes as a
//! tile-grid stream group but does not compose. [`decode_tile_grid`] decodes every tile and
//! stitches them onto the canvas, then crops to the presentation size. The probe only decodes the
//! first tile; posters (0230) and image clips (0320) use this.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

use super::decode::open_decoder;
use super::error::{MediaError, MediaErrorKind};
use super::input::Input;
use super::picture::RgbImage;

/// Larger canvases are refused instead of allocating gigabytes for a hostile file (100 MP).
const MAX_GRID_PIXELS: u64 = 100_000_000;

/// The tile grid of a file, copied out of FFmpeg's stream group.
#[derive(Debug, Clone)]
struct Grid {
    coded_width: u32,
    coded_height: u32,
    crop: (u32, u32, u32, u32),
    background: [u8; 3],
    /// (stream index in the file, x, y) of every tile.
    tiles: Vec<(usize, i64, i64)>,
}

fn read_grid(input: &Input<'_>) -> Option<Grid> {
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
    let group = groups
        .iter()
        .find(|g| g.type_ == ffi::AV_STREAM_GROUP_PARAMS_TILE_GRID && g.nb_streams > 0)?;
    // SAFETY: for a tile-grid group `params.tile_grid` is the active union member, and `streams`
    // holds `nb_streams` valid stream pointers.
    let (grid, streams) = unsafe {
        (
            &*group.params.tile_grid,
            std::slice::from_raw_parts(
                group.streams.cast::<&ffi::AVStream>(),
                group.nb_streams as usize,
            ),
        )
    };
    if grid.offsets.is_null() || grid.nb_tiles == 0 {
        return None;
    }
    // SAFETY: `offsets` holds `nb_tiles` entries (FFmpeg's contract).
    let offsets = unsafe { std::slice::from_raw_parts(grid.offsets, grid.nb_tiles as usize) };
    let tiles = offsets
        .iter()
        .filter_map(|o| {
            let stream = streams.get(o.idx as usize)?;
            Some((
                usize::try_from(stream.index).ok()?,
                i64::from(o.horizontal),
                i64::from(o.vertical),
            ))
        })
        .collect();
    let positive = |v: i32| u32::try_from(v).ok().filter(|&v| v > 0);
    let [r, g, b, _] = grid.background;
    Some(Grid {
        coded_width: positive(grid.coded_width)?,
        coded_height: positive(grid.coded_height)?,
        crop: (
            u32::try_from(grid.horizontal_offset).ok()?,
            u32::try_from(grid.vertical_offset).ok()?,
            positive(grid.width)?,
            positive(grid.height)?,
        ),
        background: [r, g, b],
        tiles,
    })
}

/// Decodes one packet with a fresh decoder for its stream: every tile is an independent picture.
fn decode_tile(
    stream: &ffi::AVStream,
    packet: &rsmpeg::avcodec::AVPacket,
) -> Result<AVFrame, MediaError> {
    let mut decoder = open_decoder(stream)?;
    decoder.send_packet(Some(packet)).ok();
    decoder.send_packet(None).ok();
    decoder
        .receive_frame()
        .map_err(|err| MediaError::new(MediaErrorKind::Damaged, format!("tile decode: {err}")))
}

fn tile_to_rgb(frame: &AVFrame) -> Result<RgbImage, MediaError> {
    super::picture::frame_to_rgb_sdr(frame, frame.width as u32, frame.height as u32)
}

/// The composed picture of a HEIF tile grid, before rotation; `Ok(None)` if `path` has no grid.
pub fn decode_tile_grid(path: &Path, cancel: &AtomicBool) -> Result<Option<RgbImage>, MediaError> {
    let mut input = Input::open(path, cancel)?;
    let Some(grid) = read_grid(&input) else {
        return Ok(None);
    };
    if u64::from(grid.coded_width) * u64::from(grid.coded_height) > MAX_GRID_PIXELS {
        return Err(MediaError::new(
            MediaErrorKind::UnsupportedFormat,
            format!(
                "tile grid {}x{} is too large",
                grid.coded_width, grid.coded_height
            ),
        ));
    }
    let mut canvas = RgbImage::filled(grid.coded_width, grid.coded_height, grid.background);
    let mut pending: Vec<(usize, i64, i64)> = grid.tiles.clone();
    while !pending.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::new(MediaErrorKind::Cancelled, "cancelled"));
        }
        let Some(packet) = input.read_packet()? else {
            break;
        };
        let index = packet.stream_index as usize;
        // A stream can be placed more than once in theory; place every occurrence.
        let places: Vec<(i64, i64)> = pending
            .iter()
            .filter(|(i, _, _)| *i == index)
            .map(|&(_, x, y)| (x, y))
            .collect();
        if places.is_empty() {
            continue;
        }
        pending.retain(|(i, _, _)| *i != index);
        let stream = input.streams()[index];
        let tile = tile_to_rgb(&decode_tile(stream, &packet)?)?;
        for (x, y) in places {
            canvas.blit(&tile, x, y);
        }
    }
    if !pending.is_empty() {
        return Err(MediaError::new(
            MediaErrorKind::Damaged,
            format!("{} grid tiles are missing", pending.len()),
        ));
    }
    let (x, y, w, h) = grid.crop;
    Ok(Some(canvas.cropped(x, y, w, h)))
}
