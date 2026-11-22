//! Decoded pictures as small 8-bit RGB images: colour conversion and scaling with `swscale`,
//! quarter-turn rotation and cropping in Rust, HDR tone mapping on the CPU, and JPEG encoding
//! with FFmpeg's native (LGPL) `mjpeg` encoder. Used by the preview generators (design §5).

use std::ffi::c_int;
use std::ptr;

use app_core::media::{HdrInfo, HdrTransfer, Rotation, VideoStreamInfo};
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::error::{MediaError, MediaErrorKind};
use super::tonemap::ToneMap;

/// A tightly packed 8-bit RGB picture (3 bytes per pixel, rows top to bottom).
#[derive(Clone, PartialEq, Eq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RgbImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgbImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl RgbImage {
    /// A `width x height` picture filled with `rgb`.
    pub fn filled(width: u32, height: u32, rgb: [u8; 3]) -> Self {
        let data = rgb.repeat(width as usize * height as usize);
        Self {
            width,
            height,
            data,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = (y as usize * self.width as usize + x as usize) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    fn set_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        let i = (y as usize * self.width as usize + x as usize) * 3;
        self.data[i..i + 3].copy_from_slice(&rgb);
    }

    /// Turns the picture clockwise by `rotation`.
    pub fn rotated(self, rotation: Rotation) -> Self {
        if rotation == Rotation::None {
            return self;
        }
        let (w, h) = (self.width, self.height);
        let (out_w, out_h) = if rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        };
        let mut out = Self::filled(out_w, out_h, [0; 3]);
        for y in 0..h {
            for x in 0..w {
                let (ox, oy) = match rotation {
                    Rotation::Cw90 => (h - 1 - y, x),
                    Rotation::Cw180 => (w - 1 - x, h - 1 - y),
                    Rotation::Cw270 => (y, w - 1 - x),
                    Rotation::None => (x, y),
                };
                out.set_pixel(ox, oy, self.pixel(x, y));
            }
        }
        out
    }

    /// The `width x height` part whose top-left corner is at (`x`, `y`), clipped to the picture.
    pub fn cropped(&self, x: u32, y: u32, width: u32, height: u32) -> Self {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let mut data = Vec::with_capacity(width as usize * height as usize * 3);
        for row in y..y + height {
            let start = (row as usize * self.width as usize + x as usize) * 3;
            data.extend_from_slice(&self.data[start..start + width as usize * 3]);
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Copies `src` into this picture with its top-left corner at (`x`, `y`), clipped.
    pub fn blit(&mut self, src: &RgbImage, x: i64, y: i64) {
        for sy in 0..src.height {
            let dy = y + i64::from(sy);
            if dy < 0 || dy >= i64::from(self.height) {
                continue;
            }
            let x0 = x.max(0);
            let x1 = (x + i64::from(src.width)).min(i64::from(self.width));
            if x0 >= x1 {
                continue;
            }
            let sx0 = (x0 - x) as usize;
            let len = (x1 - x0) as usize;
            let src_start = (sy as usize * src.width as usize + sx0) * 3;
            let dst_start = (dy as usize * self.width as usize + x0 as usize) * 3;
            self.data[dst_start..dst_start + len * 3]
                .copy_from_slice(&src.data[src_start..src_start + len * 3]);
        }
    }

    /// Scaled to exactly `width x height` (aspect not kept).
    pub fn resized(&self, width: u32, height: u32) -> Result<Self, MediaError> {
        if (width, height) == (self.width, self.height) {
            return Ok(self.clone());
        }
        let src = self.to_frame()?;
        let out = convert(&src, width, height, ffi::AV_PIX_FMT_RGB24, Colorimetry::RGB)?;
        Ok(Self::from_packed(&out, 3))
    }

    /// Scaled to cover `width x height` with the aspect kept, then cropped in the centre.
    pub fn cover(&self, width: u32, height: u32) -> Result<Self, MediaError> {
        let (sw, sh) = (u64::from(self.width), u64::from(self.height));
        let (tw, th) = (u64::from(width), u64::from(height));
        let (w, h) = if sw * th >= tw * sh {
            ((sw * th).div_ceil(sh).max(tw), th)
        } else {
            (tw, (sh * tw).div_ceil(sw).max(th))
        };
        let scaled = self.resized(w as u32, h as u32)?;
        Ok(scaled.cropped(
            (scaled.width - width) / 2,
            (scaled.height - height) / 2,
            width,
            height,
        ))
    }

    fn to_frame(&self) -> Result<AVFrame, MediaError> {
        let mut frame = alloc_frame(self.width, self.height, ffi::AV_PIX_FMT_RGB24)?;
        let stride = frame.linesize[0] as usize;
        let row = self.width as usize * 3;
        for y in 0..self.height as usize {
            // SAFETY: `alloc_frame` allocated `height` rows of `linesize[0] >= row` bytes.
            let dst =
                unsafe { std::slice::from_raw_parts_mut(frame.data_mut()[0].add(y * stride), row) };
            dst.copy_from_slice(&self.data[y * row..(y + 1) * row]);
        }
        Ok(frame)
    }

    /// Reads plane 0 of a packed frame with `bytes_per_pixel` 3 (RGB24).
    fn from_packed(frame: &AVFrame, bytes_per_pixel: usize) -> Self {
        let (width, height) = (frame.width as u32, frame.height as u32);
        let row = width as usize * bytes_per_pixel;
        let stride = frame.linesize[0] as usize;
        let mut data = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            // SAFETY: a frame of this size and format has `height` rows of `linesize[0]` bytes.
            let src = unsafe { std::slice::from_raw_parts(frame.data[0].add(y * stride), row) };
            data.extend_from_slice(src);
        }
        Self {
            width,
            height,
            data,
        }
    }
}

/// How `swscale` reads the source: matrix (`SWS_CS_*`) and range.
#[derive(Debug, Clone, Copy)]
struct Colorimetry {
    matrix: c_int,
    full_range: bool,
}

impl Colorimetry {
    const RGB: Self = Self {
        matrix: ffi::SWS_CS_DEFAULT as c_int,
        full_range: true,
    };

    /// From the frame's own tags; untagged sources follow the usual size rule (HD is BT.709).
    fn of(frame: &AVFrame) -> Self {
        let matrix = match frame.colorspace {
            ffi::AVCOL_SPC_BT709 => ffi::SWS_CS_ITU709,
            ffi::AVCOL_SPC_FCC => ffi::SWS_CS_FCC,
            ffi::AVCOL_SPC_SMPTE240M => ffi::SWS_CS_SMPTE240M,
            ffi::AVCOL_SPC_BT2020_NCL | ffi::AVCOL_SPC_BT2020_CL => ffi::SWS_CS_BT2020,
            ffi::AVCOL_SPC_BT470BG | ffi::AVCOL_SPC_SMPTE170M => ffi::SWS_CS_ITU601,
            _ if frame.height >= 720 => ffi::SWS_CS_ITU709,
            _ => ffi::SWS_CS_ITU601,
        } as c_int;
        let full_range = frame.color_range == ffi::AVCOL_RANGE_JPEG
            || matches!(
                frame.format,
                ffi::AV_PIX_FMT_YUVJ420P
                    | ffi::AV_PIX_FMT_YUVJ422P
                    | ffi::AV_PIX_FMT_YUVJ444P
                    | ffi::AV_PIX_FMT_YUVJ440P
                    | ffi::AV_PIX_FMT_YUVJ411P
            );
        Self { matrix, full_range }
    }
}

fn alloc_frame(width: u32, height: u32, format: ffi::AVPixelFormat) -> Result<AVFrame, MediaError> {
    let mut frame = AVFrame::new();
    frame.set_width(width as i32);
    frame.set_height(height as i32);
    frame.set_format(format);
    frame
        .alloc_buffer()
        .map_err(|err| MediaError::new(MediaErrorKind::Damaged, format!("frame buffer: {err}")))?;
    // Zero the whole buffers, padding included: encoders read the padding of pictures whose size
    // is not a multiple of their block size, and uninitialised bytes there would make the output
    // differ from run to run.
    for buf in frame.buf {
        if !buf.is_null() {
            // SAFETY: `buf` is a buffer of `size` bytes owned by the frame, just allocated.
            unsafe { ptr::write_bytes((*buf).data, 0, (*buf).size) };
        }
    }
    Ok(frame)
}

/// Converts and scales `src` to a new `width x height` frame of `format` (full range output).
fn convert(
    src: &AVFrame,
    width: u32,
    height: u32,
    format: ffi::AVPixelFormat,
    colorimetry: Colorimetry,
) -> Result<AVFrame, MediaError> {
    if src.width <= 0 || src.height <= 0 || width == 0 || height == 0 {
        return Err(MediaError::new(MediaErrorKind::Damaged, "empty picture"));
    }
    let failed = |what: &str| MediaError::new(MediaErrorKind::Damaged, format!("swscale: {what}"));
    let flags = ffi::SWS_BICUBIC | ffi::SWS_ACCURATE_RND | ffi::SWS_FULL_CHR_H_INT;
    // The deprecated full-range `yuvj*` formats have the same layout as `yuv*`; the range is
    // passed to swscale explicitly below (and `Colorimetry::of` already reads it from them).
    let src_format = match src.format {
        ffi::AV_PIX_FMT_YUVJ420P => ffi::AV_PIX_FMT_YUV420P,
        ffi::AV_PIX_FMT_YUVJ422P => ffi::AV_PIX_FMT_YUV422P,
        ffi::AV_PIX_FMT_YUVJ444P => ffi::AV_PIX_FMT_YUV444P,
        ffi::AV_PIX_FMT_YUVJ440P => ffi::AV_PIX_FMT_YUV440P,
        ffi::AV_PIX_FMT_YUVJ411P => ffi::AV_PIX_FMT_YUV411P,
        other => other,
    };
    // SAFETY: valid sizes and formats; checked for NULL below and freed on every path.
    let ctx = unsafe {
        ffi::sws_getContext(
            src.width,
            src.height,
            src_format,
            width as i32,
            height as i32,
            format,
            flags as c_int,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if ctx.is_null() {
        return Err(failed("unsupported conversion"));
    }
    struct Ctx(*mut ffi::SwsContext);
    impl Drop for Ctx {
        fn drop(&mut self) {
            // SAFETY: allocated by `sws_getContext` above, freed once.
            unsafe { ffi::sws_freeContext(self.0) }
        }
    }
    let ctx = Ctx(ctx);
    // SAFETY: `ctx` is valid; the coefficient tables are static. The output matrix only matters for
    // YUV output, which is always JPEG (BT.601, full range).
    unsafe {
        ffi::sws_setColorspaceDetails(
            ctx.0,
            ffi::sws_getCoefficients(colorimetry.matrix),
            c_int::from(colorimetry.full_range),
            ffi::sws_getCoefficients(ffi::SWS_CS_ITU601 as c_int),
            1,
            0,
            1 << 16,
            1 << 16,
        );
    }
    let mut dst = alloc_frame(width, height, format)?;
    // SAFETY: `src` and `dst` are allocated frames of the sizes and formats given to the context.
    let ret = unsafe {
        ffi::sws_scale(
            ctx.0,
            src.data.as_ptr().cast(),
            src.linesize.as_ptr(),
            0,
            src.height,
            dst.data_mut().as_ptr(),
            dst.linesize.as_ptr(),
        )
    };
    if ret <= 0 {
        return Err(failed("scale"));
    }
    Ok(dst)
}

/// Converts a decoded video frame to an upright-to-be RGB picture of `width x height` (before
/// rotation). HDR sources are tone-mapped to SDR (design §5 "HDR previews").
pub(crate) fn frame_to_rgb(
    frame: &AVFrame,
    width: u32,
    height: u32,
    video: &VideoStreamInfo,
) -> Result<RgbImage, MediaError> {
    let colorimetry = Colorimetry::of(frame);
    match video.hdr {
        Some(
            hdr @ HdrInfo {
                transfer: HdrTransfer::Pq | HdrTransfer::Hlg,
                ..
            },
        ) => {
            let wide = convert(frame, width, height, ffi::AV_PIX_FMT_RGB48LE, colorimetry)?;
            Ok(ToneMap::for_hdr(&hdr).apply_rgb48(&wide))
        }
        _ => frame_to_rgb_sdr(frame, width, height),
    }
}

/// Converts an SDR frame to RGB at `width x height`, with the frame's own matrix and range.
pub(crate) fn frame_to_rgb_sdr(
    frame: &AVFrame,
    width: u32,
    height: u32,
) -> Result<RgbImage, MediaError> {
    let rgb = convert(
        frame,
        width,
        height,
        ffi::AV_PIX_FMT_RGB24,
        Colorimetry::of(frame),
    )?;
    Ok(RgbImage::from_packed(&rgb, 3))
}

/// The 16-bit samples of plane 0 of an RGB48LE frame, row by row.
pub(crate) fn rgb48_rows(frame: &AVFrame) -> impl Iterator<Item = Vec<u16>> + '_ {
    let width = frame.width as usize;
    let stride = frame.linesize[0] as usize;
    (0..frame.height as usize).map(move |y| {
        // SAFETY: an RGB48LE frame of this size has `height` rows of `linesize[0]` bytes, each
        // holding at least `width * 6` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(frame.data[0].add(y * stride), width * 6) };
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_le_bytes(b))
            .collect()
    })
}

/// JPEG quality scale for FFmpeg's `mjpeg` encoder (2 = best, 31 = worst).
const JPEG_QSCALE: i32 = 4;

/// Encodes `image` as a baseline JFIF JPEG (4:2:0, full range BT.601) with FFmpeg's native
/// `mjpeg` encoder.
pub fn encode_jpeg(image: &RgbImage) -> Result<Vec<u8>, MediaError> {
    let failed = |what: &str, err: RsmpegError| {
        MediaError::new(MediaErrorKind::Damaged, format!("jpeg {what}: {err}"))
    };
    let codec = AVCodec::find_encoder_by_name(c"mjpeg")
        .ok_or_else(|| MediaError::new(MediaErrorKind::UnsupportedCodec, "no mjpeg encoder"))?;
    let mut ctx = AVCodecContext::new(&codec);
    ctx.set_width(image.width as i32);
    ctx.set_height(image.height as i32);
    ctx.set_pix_fmt(ffi::AV_PIX_FMT_YUV420P);
    ctx.set_time_base(ffi::AVRational { num: 1, den: 25 });
    ctx.set_flags(ffi::AV_CODEC_FLAG_QSCALE as i32);
    let quality = ffi::FF_QP2LAMBDA as i32 * JPEG_QSCALE;
    // SAFETY: plain fields set before `avcodec_open2`.
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).global_quality = quality;
        (*raw).color_range = ffi::AVCOL_RANGE_JPEG;
        (*raw).thread_count = 1;
    }
    ctx.open(None).map_err(|err| failed("open", err))?;

    let src = image.to_frame()?;
    let mut yuv = convert(
        &src,
        image.width,
        image.height,
        ffi::AV_PIX_FMT_YUV420P,
        Colorimetry::RGB,
    )?;
    yuv.set_pts(0);
    // SAFETY: plain fields of an owned frame.
    unsafe {
        let raw = yuv.as_mut_ptr();
        (*raw).quality = quality;
        (*raw).color_range = ffi::AVCOL_RANGE_JPEG;
    }
    ctx.send_frame(Some(&yuv))
        .map_err(|err| failed("encode", err))?;
    ctx.send_frame(None).map_err(|err| failed("flush", err))?;
    let packet = ctx.receive_packet().map_err(|err| failed("packet", err))?;
    // SAFETY: an encoded packet holds `size` bytes at `data`.
    let bytes = unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
    Ok(bytes.to_vec())
}

/// `(width, height)` of a picture whose long side is `long_side`, with the aspect of `w x h`.
pub fn fit_long_side(w: u32, h: u32, long_side: u32) -> (u32, u32) {
    let (w, h) = (u64::from(w.max(1)), u64::from(h.max(1)));
    let long = u64::from(long_side);
    let short = |a: u64, b: u64| ((a * long * 2 + b) / (2 * b)).max(1) as u32;
    if w >= h {
        (long_side, short(h, w))
    } else {
        (short(w, h), long_side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 picture whose pixels encode their own coordinates.
    fn marked() -> RgbImage {
        let mut image = RgbImage::filled(3, 2, [0; 3]);
        for y in 0..2 {
            for x in 0..3 {
                image.set_pixel(x, y, [x as u8, y as u8, 7]);
            }
        }
        image
    }

    #[test]
    fn rotation_turns_clockwise() {
        let image = marked();
        let cw90 = image.clone().rotated(Rotation::Cw90);
        assert_eq!((cw90.width, cw90.height), (2, 3));
        // The top-left pixel moves to the top-right corner.
        assert_eq!(cw90.pixel(1, 0), [0, 0, 7]);
        assert_eq!(cw90.pixel(0, 0), [0, 1, 7]);
        let cw180 = image.clone().rotated(Rotation::Cw180);
        assert_eq!(cw180.pixel(2, 1), [0, 0, 7]);
        let cw270 = image.clone().rotated(Rotation::Cw270);
        assert_eq!((cw270.width, cw270.height), (2, 3));
        assert_eq!(cw270.pixel(0, 2), [0, 0, 7]);
        assert_eq!(image.clone().rotated(Rotation::None), image);
        // Four quarter turns are the identity.
        let back = image
            .clone()
            .rotated(Rotation::Cw90)
            .rotated(Rotation::Cw90)
            .rotated(Rotation::Cw90)
            .rotated(Rotation::Cw90);
        assert_eq!(back, image);
    }

    #[test]
    fn crop_and_blit_clip_to_the_picture() {
        let image = marked();
        let part = image.cropped(1, 1, 5, 5);
        assert_eq!((part.width, part.height), (2, 1));
        assert_eq!(part.pixel(0, 0), [1, 1, 7]);

        let mut canvas = RgbImage::filled(4, 4, [9; 3]);
        canvas.blit(&image, 2, -1);
        assert_eq!(canvas.pixel(2, 0), [0, 1, 7]);
        assert_eq!(canvas.pixel(3, 0), [1, 1, 7]);
        assert_eq!(canvas.pixel(0, 0), [9; 3]);
        assert_eq!(canvas.pixel(2, 1), [9; 3]);
    }

    #[test]
    fn long_side_fit_keeps_the_aspect() {
        assert_eq!(fit_long_side(1920, 1080, 320), (320, 180));
        assert_eq!(fit_long_side(1080, 1920, 320), (180, 320));
        assert_eq!(fit_long_side(64, 64, 320), (320, 320));
        assert_eq!(fit_long_side(10_000, 1, 320), (320, 1));
    }

    #[test]
    fn resize_cover_and_jpeg() {
        let image = RgbImage::filled(64, 48, [200, 40, 40]);
        let cover = image.cover(54, 96).unwrap();
        assert_eq!((cover.width, cover.height), (54, 96));
        let px = cover.pixel(20, 50);
        assert!(px[0] > 180 && px[1] < 60, "{px:?}");

        let jpeg = encode_jpeg(&cover).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8], "SOI marker");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9], "EOI marker");
        assert_eq!(encode_jpeg(&cover).unwrap(), jpeg, "deterministic");
    }
}
