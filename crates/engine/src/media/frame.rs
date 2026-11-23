//! Decoded video frames as the render graph reads them (design `docs/design/render-graph.md`
//! §5.3, `docs/design/media-import.md` §5).
//!
//! A software-decoded `AVFrame` becomes a [`CpuFrame`] in one of the few layouts the
//! source-convert shader samples directly (planar or semi-planar Y'CbCr, packed RGBA, 8 or 16 bits
//! per sample). Every other pixel format goes through `sws_scale` first, on the decode thread
//! ([`CpuFrameConverter`]).

#![allow(
    dead_code,
    reason = "used by the compositor and the readers, tasks 0320 steps 4 and 5"
)]

use std::ffi::{CStr, CString, c_int};
use std::ptr;

use app_core::Time;
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

use super::error::{MediaError, MediaErrorKind};

/// One decoded video frame and its media time.
#[derive(Debug)]
pub(crate) struct DecodedVideoFrame {
    pub pts: Time,
    pub data: FrameData,
}

/// Where the pixels of a decoded frame live.
#[derive(Debug)]
pub(crate) enum FrameData {
    /// Software decode, or hardware decode with download (the per-stream fallback, §5.4).
    Cpu(CpuFrame),
    // `Gpu(GpuFrame)`: a hardware-decoded NV12/P010 surface on the shared device, added by the
    // D3D12VA zero-copy module (`media::d3d12`, render-graph.md §5.3 and §5.4). The convert pass
    // samples it through `TextureAspect::Plane0` / `Plane1` views (`render::convert`).
}

/// How the planes of a [`CpuFrame`] are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlaneLayout {
    /// Y', Cb and Cr in three planes; chroma planes are `chroma_w x chroma_h` samples.
    Planar3 { chroma_w: u32, chroma_h: u32 },
    /// Y' in plane 0, interleaved CbCr in plane 1, 4:2:0 (NV12, P010).
    SemiPlanar,
    /// R, G, B, A interleaved in plane 0 (straight alpha).
    Packed,
}

/// Storage of one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SampleDepth {
    U8,
    /// A 16-bit little-endian container holding `bits` significant bits, either in the high bits
    /// (`msb_aligned`, P010) or in the low bits (`yuv420p10le`).
    U16 {
        bits: u8,
        msb_aligned: bool,
    },
}

impl SampleDepth {
    /// Significant bits of a code value.
    pub fn bits(self) -> u8 {
        match self {
            Self::U8 => 8,
            Self::U16 { bits, .. } => bits,
        }
    }

    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 { .. } => 2,
        }
    }

    /// The factor from a sample read as unorm (`0..=1`) to its code value (§5.2 step 1): 255,
    /// `65535 / 2^(16 - bits)` for MSB-aligned and 65535 for LSB-aligned 16-bit containers.
    pub fn code_scale(self) -> f32 {
        match self {
            Self::U8 => 255.0,
            Self::U16 {
                bits,
                msb_aligned: true,
            } => 65535.0 / f32::from(1u16 << (16 - bits.clamp(1, 16))),
            Self::U16 {
                msb_aligned: false, ..
            } => 65535.0,
        }
    }
}

/// One plane: `stride` bytes per row, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PlaneBuf {
    pub stride: usize,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for PlaneBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaneBuf")
            .field("stride", &self.stride)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// A decoded picture in CPU memory, in a layout the convert shader reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CpuFrame {
    pub layout: PlaneLayout,
    pub depth: SampleDepth,
    /// The picture size (FFmpeg has already cropped the coded padding).
    pub visible: (u32, u32),
    pub planes: [Option<PlaneBuf>; 3],
}

/// A [`CpuFrame`] whose planes do not match its layout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FrameError {
    #[error("the picture is empty")]
    Empty,
    #[error("plane {0} is missing")]
    MissingPlane(usize),
    #[error("plane {0} is not used by this layout")]
    UnexpectedPlane(usize),
    #[error("plane {plane}: {len} bytes cannot hold {rows} rows of {row_bytes} at stride {stride}")]
    PlaneTooSmall {
        plane: usize,
        stride: usize,
        row_bytes: usize,
        rows: u32,
        len: usize,
    },
}

impl PlaneLayout {
    /// Number of planes.
    pub fn planes(self) -> usize {
        match self {
            Self::Planar3 { .. } => 3,
            Self::SemiPlanar => 2,
            Self::Packed => 1,
        }
    }

    /// Samples (channels) per texel of `plane`.
    pub fn channels(self, plane: usize) -> u32 {
        match (self, plane) {
            (Self::Packed, _) => 4,
            (Self::SemiPlanar, 1) => 2,
            _ => 1,
        }
    }

    /// Size in texels of `plane` for a `visible` picture.
    pub fn plane_size(self, plane: usize, visible: (u32, u32)) -> (u32, u32) {
        match (self, plane) {
            (_, 0) => visible,
            (Self::Planar3 { chroma_w, chroma_h }, _) => (chroma_w, chroma_h),
            _ => (visible.0.div_ceil(2), visible.1.div_ceil(2)),
        }
    }

    /// How many luma samples share one chroma sample, horizontally and vertically (1, 2 or 4;
    /// 1 for packed RGBA).
    pub fn subsampling(self, visible: (u32, u32)) -> (u32, u32) {
        match self {
            Self::Planar3 { chroma_w, chroma_h } => (
                subsampling(visible.0, chroma_w),
                subsampling(visible.1, chroma_h),
            ),
            Self::SemiPlanar => (2, 2),
            Self::Packed => (1, 1),
        }
    }
}

/// The factor `s` in {1, 2, 4} with `ceil(luma / s) == chroma` (FFmpeg's `AV_CEIL_RSHIFT`), else
/// the rounded ratio.
fn subsampling(luma: u32, chroma: u32) -> u32 {
    [1, 2, 4]
        .into_iter()
        .find(|&s| luma.div_ceil(s) == chroma)
        .unwrap_or_else(|| (luma as f32 / chroma.max(1) as f32).round().max(1.0) as u32)
}

impl CpuFrame {
    /// Bytes of one row of `plane` without padding.
    pub fn row_bytes(&self, plane: usize) -> usize {
        let (w, _) = self.layout.plane_size(plane, self.visible);
        w as usize * self.layout.channels(plane) as usize * self.depth.bytes()
    }

    /// Checks that every plane the layout needs is present and large enough.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.visible.0 == 0 || self.visible.1 == 0 {
            return Err(FrameError::Empty);
        }
        for (index, plane) in self.planes.iter().enumerate() {
            let used = index < self.layout.planes();
            let Some(plane) = plane else {
                if used {
                    return Err(FrameError::MissingPlane(index));
                }
                continue;
            };
            if !used {
                return Err(FrameError::UnexpectedPlane(index));
            }
            let row_bytes = self.row_bytes(index);
            let (_, rows) = self.layout.plane_size(index, self.visible);
            let needed = plane.stride * (rows as usize - 1) + row_bytes;
            if plane.stride < row_bytes || plane.bytes.len() < needed {
                return Err(FrameError::PlaneTooSmall {
                    plane: index,
                    stride: plane.stride,
                    row_bytes,
                    rows,
                    len: plane.bytes.len(),
                });
            }
        }
        Ok(())
    }
}

/// What happens to a decoded pixel format before the convert shader sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CpuTarget {
    /// Read as it is: the planes are copied.
    Native {
        layout: NativeLayout,
        depth: SampleDepth,
    },
    /// Converted with `sws_scale` to this format (which is always `Native`) first.
    Scale(ffi::AVPixelFormat),
}

/// [`PlaneLayout`] without the chroma size, which depends on the picture size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeLayout {
    Planar3 {
        log2_chroma_w: u8,
        log2_chroma_h: u8,
    },
    SemiPlanar,
    Packed,
}

impl NativeLayout {
    fn for_size(self, (w, h): (u32, u32)) -> PlaneLayout {
        match self {
            Self::Planar3 {
                log2_chroma_w,
                log2_chroma_h,
            } => PlaneLayout::Planar3 {
                chroma_w: w.div_ceil(1 << log2_chroma_w),
                chroma_h: h.div_ceil(1 << log2_chroma_h),
            },
            Self::SemiPlanar => PlaneLayout::SemiPlanar,
            Self::Packed => PlaneLayout::Packed,
        }
    }
}

fn descriptor(format: ffi::AVPixelFormat) -> Option<&'static ffi::AVPixFmtDescriptor> {
    // SAFETY: returns NULL or a pointer to FFmpeg's static descriptor table.
    unsafe { ffi::av_pix_fmt_desc_get(format).as_ref() }
}

/// How the reader normalises `format`; `None` for hardware surfaces and unknown formats.
pub(crate) fn cpu_target(format: ffi::AVPixelFormat) -> Option<CpuTarget> {
    let desc = descriptor(format)?;
    let flag = |f: u32| desc.flags & u64::from(f) != 0;
    if flag(ffi::AV_PIX_FMT_FLAG_HWACCEL) || desc.nb_components == 0 {
        return None;
    }
    let depth = desc.comp[0].depth.clamp(1, 16) as u8;
    let u16_lsb = SampleDepth::U16 {
        bits: depth,
        msb_aligned: false,
    };
    let native = |layout, depth| Some(CpuTarget::Native { layout, depth });
    match format {
        ffi::AV_PIX_FMT_NV12 => return native(NativeLayout::SemiPlanar, SampleDepth::U8),
        ffi::AV_PIX_FMT_P010LE | ffi::AV_PIX_FMT_P012LE | ffi::AV_PIX_FMT_P016LE => {
            let depth = SampleDepth::U16 {
                bits: depth,
                msb_aligned: true,
            };
            return native(NativeLayout::SemiPlanar, depth);
        }
        ffi::AV_PIX_FMT_RGBA => return native(NativeLayout::Packed, SampleDepth::U8),
        ffi::AV_PIX_FMT_RGBA64LE => return native(NativeLayout::Packed, u16_lsb),
        _ => {}
    }
    let rgb = flag(ffi::AV_PIX_FMT_FLAG_RGB)
        || flag(ffi::AV_PIX_FMT_FLAG_PAL)
        || flag(ffi::AV_PIX_FMT_FLAG_BAYER)
        || flag(ffi::AV_PIX_FMT_FLAG_XYZ);
    let alpha = flag(ffi::AV_PIX_FMT_FLAG_ALPHA);
    let comp = &desc.comp;
    // Planar Y'CbCr, little-endian, one plane per component, no padding bits: any subsampling.
    let planar_yuv = !rgb
        && !alpha
        && !flag(ffi::AV_PIX_FMT_FLAG_BE)
        && !flag(ffi::AV_PIX_FMT_FLAG_FLOAT)
        && flag(ffi::AV_PIX_FMT_FLAG_PLANAR)
        && desc.nb_components == 3
        && (0..3).all(|c| {
            comp[c].plane == c as c_int
                && comp[c].shift == 0
                && comp[c].offset == 0
                && comp[c].depth == comp[0].depth
                && comp[c].step == if depth > 8 { 2 } else { 1 }
        })
        && desc.log2_chroma_w <= 2
        && desc.log2_chroma_h <= 2;
    if planar_yuv && depth >= 8 {
        let layout = NativeLayout::Planar3 {
            log2_chroma_w: desc.log2_chroma_w,
            log2_chroma_h: desc.log2_chroma_h,
        };
        return native(layout, if depth > 8 { u16_lsb } else { SampleDepth::U8 });
    }
    let wide = depth > 8;
    let target = if rgb || alpha {
        if wide {
            ffi::AV_PIX_FMT_RGBA64LE
        } else {
            ffi::AV_PIX_FMT_RGBA
        }
    } else if desc.nb_components == 1 {
        // Grey: luma only, neutral chroma at the lowest chroma resolution.
        if wide {
            ffi::AV_PIX_FMT_YUV420P16LE
        } else {
            ffi::AV_PIX_FMT_YUV420P
        }
    } else if wide {
        ffi::AV_PIX_FMT_YUV444P16LE
    } else {
        ffi::AV_PIX_FMT_YUV444P
    };
    Some(CpuTarget::Scale(target))
}

/// True when frames of the FFmpeg pixel format named `pixel_format` reach the shader as RGBA
/// (RGB, palettised, and anything with alpha), false for Y'CbCr and unknown names.
pub(crate) fn decodes_to_rgb(pixel_format: &str) -> bool {
    let Ok(name) = CString::new(pixel_format) else {
        return false;
    };
    // SAFETY: `name` is a valid NUL-terminated string.
    let format = unsafe { ffi::av_get_pix_fmt(name.as_ptr()) };
    let target = match cpu_target(format) {
        Some(CpuTarget::Scale(target)) => cpu_target(target),
        other => other,
    };
    matches!(
        target,
        Some(CpuTarget::Native {
            layout: NativeLayout::Packed,
            ..
        })
    )
}

fn damaged(detail: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorKind::Damaged, detail)
}

/// Turns decoded `AVFrame`s into [`CpuFrame`]s, keeping one `sws_scale` context for formats the
/// shader does not read. One per reader (decode thread).
#[derive(Debug, Default)]
pub(crate) struct CpuFrameConverter {
    scaler: Option<Scaler>,
}

impl CpuFrameConverter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The planes of a software-decoded `frame`, converted when its format needs it.
    pub fn convert(&mut self, frame: &AVFrame) -> Result<CpuFrame, MediaError> {
        if frame.width <= 0 || frame.height <= 0 {
            return Err(damaged("empty picture"));
        }
        let visible = (frame.width as u32, frame.height as u32);
        match cpu_target(frame.format) {
            None => Err(MediaError::new(
                MediaErrorKind::UnsupportedCodec,
                format!(
                    "pixel format {} cannot be read",
                    pixel_format_name(frame.format)
                ),
            )),
            Some(CpuTarget::Native { layout, depth }) => {
                Ok(copy_planes(frame, layout.for_size(visible), depth))
            }
            Some(CpuTarget::Scale(target)) => {
                let Some(CpuTarget::Native { layout, depth }) = cpu_target(target) else {
                    unreachable!("scale targets are native formats");
                };
                let scaler = match &mut self.scaler {
                    Some(scaler) if scaler.matches(frame, target) => scaler,
                    slot => slot.insert(Scaler::new(frame, target)?),
                };
                let scaled = scaler.scale(frame)?;
                Ok(copy_planes(scaled, layout.for_size(visible), depth))
            }
        }
    }
}

fn pixel_format_name(format: ffi::AVPixelFormat) -> String {
    // SAFETY: returns NULL or a static NUL-terminated string.
    let name = unsafe { ffi::av_get_pix_fmt_name(format) };
    if name.is_null() {
        return format!("#{format}");
    }
    // SAFETY: checked for NULL above; FFmpeg's names are static.
    unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned()
}

/// Copies the planes of `frame` (whose format has `layout` and `depth`) into tight rows.
fn copy_planes(frame: &AVFrame, layout: PlaneLayout, depth: SampleDepth) -> CpuFrame {
    let visible = (frame.width as u32, frame.height as u32);
    let mut out = CpuFrame {
        layout,
        depth,
        visible,
        planes: [None, None, None],
    };
    for index in 0..layout.planes() {
        let row = out.row_bytes(index);
        let (_, rows) = layout.plane_size(index, visible);
        let stride = frame.linesize[index] as isize;
        let mut bytes = Vec::with_capacity(row * rows as usize);
        for y in 0..rows as isize {
            // SAFETY: a decoded frame of this format and size has `rows` rows of at least `row`
            // bytes in plane `index`, `linesize` apart (negative for bottom-up pictures).
            let src =
                unsafe { std::slice::from_raw_parts(frame.data[index].offset(y * stride), row) };
            bytes.extend_from_slice(src);
        }
        out.planes[index] = Some(PlaneBuf { stride: row, bytes });
    }
    out
}

/// A `sws_scale` context for one source format and size, and its reused output frame.
struct Scaler {
    ctx: *mut ffi::SwsContext,
    src: (
        ffi::AVPixelFormat,
        c_int,
        c_int,
        ffi::AVColorSpace,
        ffi::AVColorRange,
    ),
    dst: AVFrame,
}

impl std::fmt::Debug for Scaler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scaler")
            .field("src", &self.src)
            .finish_non_exhaustive()
    }
}

// SAFETY: the context is owned by this value and only used through `&mut self`; swscale contexts
// carry no thread affinity.
unsafe impl Send for Scaler {}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: allocated by `sws_getContext`, freed once.
        unsafe { ffi::sws_freeContext(self.ctx) }
    }
}

impl Scaler {
    fn key(
        frame: &AVFrame,
    ) -> (
        ffi::AVPixelFormat,
        c_int,
        c_int,
        ffi::AVColorSpace,
        ffi::AVColorRange,
    ) {
        (
            frame.format,
            frame.width,
            frame.height,
            frame.colorspace,
            frame.color_range,
        )
    }

    fn matches(&self, frame: &AVFrame, target: ffi::AVPixelFormat) -> bool {
        self.src == Self::key(frame) && self.dst.format == target
    }

    fn new(frame: &AVFrame, target: ffi::AVPixelFormat) -> Result<Self, MediaError> {
        let src_format = match frame.format {
            ffi::AV_PIX_FMT_YUVJ420P => ffi::AV_PIX_FMT_YUV420P,
            ffi::AV_PIX_FMT_YUVJ422P => ffi::AV_PIX_FMT_YUV422P,
            ffi::AV_PIX_FMT_YUVJ444P => ffi::AV_PIX_FMT_YUV444P,
            ffi::AV_PIX_FMT_YUVJ440P => ffi::AV_PIX_FMT_YUV440P,
            ffi::AV_PIX_FMT_YUVJ411P => ffi::AV_PIX_FMT_YUV411P,
            other => other,
        };
        let flags = ffi::SWS_POINT | ffi::SWS_ACCURATE_RND | ffi::SWS_FULL_CHR_H_INT;
        // SAFETY: valid sizes and formats; checked for NULL below and freed in `Drop`.
        let ctx = unsafe {
            ffi::sws_getContext(
                frame.width,
                frame.height,
                src_format,
                frame.width,
                frame.height,
                target,
                flags as c_int,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if ctx.is_null() {
            return Err(MediaError::new(
                MediaErrorKind::UnsupportedCodec,
                format!(
                    "swscale cannot convert {} to {}",
                    pixel_format_name(frame.format),
                    pixel_format_name(target)
                ),
            ));
        }
        let mut dst = AVFrame::new();
        dst.set_width(frame.width);
        dst.set_height(frame.height);
        dst.set_format(target);
        let scaler = Self {
            ctx,
            src: Self::key(frame),
            dst,
        };
        scaler.set_colorimetry(frame, target);
        let mut scaler = scaler;
        scaler
            .dst
            .alloc_buffer()
            .map_err(|err| damaged(format!("frame buffer: {err}")))?;
        Ok(scaler)
    }

    /// Y'CbCr to Y'CbCr keeps the code values (same range in and out), so the convert pass applies
    /// the stream's range and matrix as for native formats; Y'CbCr to RGBA uses the frame's matrix
    /// and range and writes full-range RGB.
    fn set_colorimetry(&self, frame: &AVFrame, target: ffi::AVPixelFormat) {
        let rgb_out = matches!(target, ffi::AV_PIX_FMT_RGBA | ffi::AV_PIX_FMT_RGBA64LE);
        let src_rgb = descriptor(frame.format).is_some_and(|d| {
            d.flags & u64::from(ffi::AV_PIX_FMT_FLAG_RGB | ffi::AV_PIX_FMT_FLAG_PAL) != 0
        });
        let matrix = match frame.colorspace {
            ffi::AVCOL_SPC_BT709 | ffi::AVCOL_SPC_SMPTE240M => ffi::SWS_CS_ITU709,
            ffi::AVCOL_SPC_BT2020_NCL | ffi::AVCOL_SPC_BT2020_CL => ffi::SWS_CS_BT2020,
            ffi::AVCOL_SPC_BT470BG | ffi::AVCOL_SPC_SMPTE170M | ffi::AVCOL_SPC_FCC => {
                ffi::SWS_CS_ITU601
            }
            _ if frame.height >= 720 => ffi::SWS_CS_ITU709,
            _ => ffi::SWS_CS_ITU601,
        } as c_int;
        let name = pixel_format_name(frame.format);
        let full = match frame.color_range {
            ffi::AVCOL_RANGE_JPEG => true,
            ffi::AVCOL_RANGE_MPEG => false,
            _ => {
                src_rgb
                    || name.starts_with("yuvj")
                    || name.starts_with("gray")
                    || name.starts_with("ya")
            }
        };
        let dst_full = if rgb_out { true } else { full };
        // SAFETY: `ctx` is valid; the coefficient tables are static.
        unsafe {
            ffi::sws_setColorspaceDetails(
                self.ctx,
                ffi::sws_getCoefficients(matrix),
                c_int::from(full),
                ffi::sws_getCoefficients(matrix),
                c_int::from(dst_full),
                0,
                1 << 16,
                1 << 16,
            );
        }
    }

    fn scale(&mut self, frame: &AVFrame) -> Result<&AVFrame, MediaError> {
        // SAFETY: `frame` and `dst` are allocated frames of the sizes and formats given to the
        // context.
        let ret = unsafe {
            ffi::sws_scale(
                self.ctx,
                frame.data.as_ptr().cast(),
                frame.linesize.as_ptr(),
                0,
                frame.height,
                self.dst.data_mut().as_ptr(),
                self.dst.linesize.as_ptr(),
            )
        };
        if ret <= 0 {
            return Err(damaged("swscale failed"));
        }
        Ok(&self.dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSB10: SampleDepth = SampleDepth::U16 {
        bits: 10,
        msb_aligned: false,
    };
    const P010: SampleDepth = SampleDepth::U16 {
        bits: 10,
        msb_aligned: true,
    };

    #[test]
    fn code_scales() {
        assert_eq!(SampleDepth::U8.code_scale(), 255.0);
        assert_eq!(P010.code_scale(), 65535.0 / 64.0);
        assert_eq!(LSB10.code_scale(), 65535.0);
        // The P010 code of 1023 is stored as 1023 << 6.
        assert_eq!(
            f32::from(1023u16 << 6) / 65535.0 * P010.code_scale(),
            1023.0
        );
    }

    #[test]
    fn plane_sizes_and_subsampling() {
        let visible = (1921, 1081);
        let yuv420 = NativeLayout::Planar3 {
            log2_chroma_w: 1,
            log2_chroma_h: 1,
        }
        .for_size(visible);
        assert_eq!(
            yuv420,
            PlaneLayout::Planar3 {
                chroma_w: 961,
                chroma_h: 541
            }
        );
        assert_eq!(yuv420.plane_size(0, visible), visible);
        assert_eq!(yuv420.plane_size(2, visible), (961, 541));
        assert_eq!(yuv420.subsampling(visible), (2, 2));
        let yuv411 = NativeLayout::Planar3 {
            log2_chroma_w: 2,
            log2_chroma_h: 0,
        }
        .for_size((720, 480));
        assert_eq!(yuv411.subsampling((720, 480)), (4, 1));
        assert_eq!(PlaneLayout::SemiPlanar.plane_size(1, (5, 3)), (3, 2));
        assert_eq!(PlaneLayout::SemiPlanar.channels(1), 2);
        assert_eq!(PlaneLayout::Packed.channels(0), 4);
        assert_eq!(PlaneLayout::Packed.subsampling((7, 7)), (1, 1));
    }

    fn frame(layout: PlaneLayout, depth: SampleDepth, visible: (u32, u32)) -> CpuFrame {
        let mut frame = CpuFrame {
            layout,
            depth,
            visible,
            planes: [None, None, None],
        };
        for index in 0..layout.planes() {
            let row = frame.row_bytes(index);
            let rows = layout.plane_size(index, visible).1 as usize;
            frame.planes[index] = Some(PlaneBuf {
                stride: row + 4,
                bytes: vec![0; (row + 4) * rows],
            });
        }
        frame
    }

    #[test]
    fn validation() {
        let nv12 = frame(PlaneLayout::SemiPlanar, P010, (6, 4));
        assert_eq!(nv12.row_bytes(0), 12);
        assert_eq!(nv12.row_bytes(1), 12);
        assert_eq!(nv12.validate(), Ok(()));
        let packed = frame(PlaneLayout::Packed, SampleDepth::U8, (3, 2));
        assert_eq!(packed.row_bytes(0), 12);
        assert_eq!(packed.validate(), Ok(()));

        let mut missing = nv12.clone();
        missing.planes[1] = None;
        assert_eq!(missing.validate(), Err(FrameError::MissingPlane(1)));
        let mut extra = packed.clone();
        extra.planes[2] = extra.planes[0].clone();
        assert_eq!(extra.validate(), Err(FrameError::UnexpectedPlane(2)));
        let mut short = nv12.clone();
        short.planes[0].as_mut().unwrap().bytes.truncate(40);
        assert!(matches!(
            short.validate(),
            Err(FrameError::PlaneTooSmall { plane: 0, .. })
        ));
        let mut empty = packed;
        empty.visible = (0, 2);
        assert_eq!(empty.validate(), Err(FrameError::Empty));
    }

    #[test]
    fn pixel_formats_map_to_shader_layouts() {
        let native = |f| match cpu_target(f) {
            Some(CpuTarget::Native { layout, depth }) => Some((layout, depth)),
            _ => None,
        };
        let planar = |w, h| NativeLayout::Planar3 {
            log2_chroma_w: w,
            log2_chroma_h: h,
        };
        assert_eq!(
            native(ffi::AV_PIX_FMT_YUV420P),
            Some((planar(1, 1), SampleDepth::U8))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_YUVJ422P),
            Some((planar(1, 0), SampleDepth::U8))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_YUV420P10LE),
            Some((planar(1, 1), LSB10))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_YUV444P12LE),
            Some((
                planar(0, 0),
                SampleDepth::U16 {
                    bits: 12,
                    msb_aligned: false
                }
            ))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_NV12),
            Some((NativeLayout::SemiPlanar, SampleDepth::U8))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_P010LE),
            Some((NativeLayout::SemiPlanar, P010))
        );
        assert_eq!(
            native(ffi::AV_PIX_FMT_RGBA),
            Some((NativeLayout::Packed, SampleDepth::U8))
        );
        let scaled = |f| match cpu_target(f) {
            Some(CpuTarget::Scale(target)) => Some(target),
            _ => None,
        };
        assert_eq!(scaled(ffi::AV_PIX_FMT_PAL8), Some(ffi::AV_PIX_FMT_RGBA));
        assert_eq!(scaled(ffi::AV_PIX_FMT_RGB24), Some(ffi::AV_PIX_FMT_RGBA));
        assert_eq!(
            scaled(ffi::AV_PIX_FMT_RGB48LE),
            Some(ffi::AV_PIX_FMT_RGBA64LE)
        );
        assert_eq!(
            scaled(ffi::AV_PIX_FMT_GBRP10LE),
            Some(ffi::AV_PIX_FMT_RGBA64LE)
        );
        assert_eq!(scaled(ffi::AV_PIX_FMT_YUVA420P), Some(ffi::AV_PIX_FMT_RGBA));
        assert_eq!(
            scaled(ffi::AV_PIX_FMT_YUYV422),
            Some(ffi::AV_PIX_FMT_YUV444P)
        );
        assert_eq!(scaled(ffi::AV_PIX_FMT_NV21), Some(ffi::AV_PIX_FMT_YUV444P));
        assert_eq!(scaled(ffi::AV_PIX_FMT_GRAY8), Some(ffi::AV_PIX_FMT_YUV420P));
        assert_eq!(
            scaled(ffi::AV_PIX_FMT_GRAY16LE),
            Some(ffi::AV_PIX_FMT_YUV420P16LE)
        );
        assert_eq!(
            scaled(ffi::AV_PIX_FMT_YUV420P10BE),
            Some(ffi::AV_PIX_FMT_YUV444P16LE)
        );
        // Every scale target is read natively.
        for target in [
            ffi::AV_PIX_FMT_RGBA,
            ffi::AV_PIX_FMT_RGBA64LE,
            ffi::AV_PIX_FMT_YUV420P,
            ffi::AV_PIX_FMT_YUV420P16LE,
            ffi::AV_PIX_FMT_YUV444P,
            ffi::AV_PIX_FMT_YUV444P16LE,
        ] {
            assert!(native(target).is_some(), "{target}");
        }
        assert_eq!(cpu_target(ffi::AV_PIX_FMT_D3D12), None);
        assert!(decodes_to_rgb("rgb24") && decodes_to_rgb("pal8") && decodes_to_rgb("rgba"));
        assert!(decodes_to_rgb("yuva420p"));
        assert!(
            !decodes_to_rgb("yuv420p10le") && !decodes_to_rgb("gray") && !decodes_to_rgb("nope")
        );
    }

    fn av_frame(format: ffi::AVPixelFormat, w: i32, h: i32) -> AVFrame {
        let mut frame = AVFrame::new();
        frame.set_width(w);
        frame.set_height(h);
        frame.set_format(format);
        frame.alloc_buffer().unwrap();
        frame
    }

    /// Writes `value(x, y)` into the 16-bit samples of plane `plane`.
    fn fill16(
        frame: &mut AVFrame,
        plane: usize,
        w: usize,
        h: usize,
        value: impl Fn(usize, usize) -> u16,
    ) {
        let stride = frame.linesize[plane] as usize;
        for y in 0..h {
            for x in 0..w {
                let bytes = value(x, y).to_le_bytes();
                // SAFETY: inside the plane of an allocated frame (`w`/`h` are the plane's size).
                unsafe {
                    let p = frame.data_mut()[plane].add(y * stride + x * 2);
                    *p = bytes[0];
                    *p.add(1) = bytes[1];
                }
            }
        }
    }

    #[test]
    fn native_frames_are_copied_into_tight_planes() {
        let mut frame = av_frame(ffi::AV_PIX_FMT_YUV420P10LE, 5, 3);
        fill16(&mut frame, 0, 5, 3, |x, y| (x + 10 * y) as u16);
        fill16(&mut frame, 1, 3, 2, |x, _| 500 + x as u16);
        fill16(&mut frame, 2, 3, 2, |_, y| 600 + y as u16);
        let cpu = CpuFrameConverter::new().convert(&frame).unwrap();
        assert_eq!(cpu.validate(), Ok(()));
        assert_eq!(cpu.depth, LSB10);
        assert_eq!(
            cpu.layout,
            PlaneLayout::Planar3 {
                chroma_w: 3,
                chroma_h: 2
            }
        );
        let y = cpu.planes[0].as_ref().unwrap();
        assert_eq!(y.stride, 10);
        assert_eq!(
            &y.bytes[10..14],
            &[10, 0, 11, 0],
            "row 1 starts with 10, 11"
        );
        let cr = cpu.planes[2].as_ref().unwrap();
        assert_eq!(&cr.bytes[6..8], &601u16.to_le_bytes());
    }

    #[test]
    fn other_formats_are_scaled_to_a_native_layout() {
        let mut frame = av_frame(ffi::AV_PIX_FMT_RGB24, 4, 2);
        let stride = frame.linesize[0] as usize;
        for y in 0..2 {
            for x in 0..4 {
                // SAFETY: inside plane 0 of the allocated 4x2 RGB24 frame.
                unsafe {
                    let p = frame.data_mut()[0].add(y * stride + x * 3);
                    *p = 200;
                    *p.add(1) = (x * 50) as u8;
                    *p.add(2) = 7;
                }
            }
        }
        let mut converter = CpuFrameConverter::new();
        let cpu = converter.convert(&frame).unwrap();
        assert_eq!(
            (cpu.layout, cpu.depth),
            (PlaneLayout::Packed, SampleDepth::U8)
        );
        assert_eq!(cpu.validate(), Ok(()));
        let rgba = &cpu.planes[0].as_ref().unwrap().bytes;
        assert_eq!(&rgba[8..12], &[200, 100, 7, 255]);
        // The scaler is reused for the next frame of the same shape.
        let again = converter.convert(&frame).unwrap();
        assert_eq!(again, cpu);

        // Grey keeps its code values and gets neutral chroma.
        let mut gray = av_frame(ffi::AV_PIX_FMT_GRAY8, 4, 4);
        let stride = gray.linesize[0] as usize;
        for y in 0..4 {
            for x in 0..4 {
                // SAFETY: inside plane 0 of the allocated 4x4 GRAY8 frame.
                unsafe { *gray.data_mut()[0].add(y * stride + x) = (x * 60 + 10) as u8 };
            }
        }
        let cpu = converter.convert(&gray).unwrap();
        assert_eq!(
            cpu.layout,
            PlaneLayout::Planar3 {
                chroma_w: 2,
                chroma_h: 2
            }
        );
        assert_eq!(
            &cpu.planes[0].as_ref().unwrap().bytes[..4],
            &[10, 70, 130, 190]
        );
        assert_eq!(cpu.planes[1].as_ref().unwrap().bytes, vec![128; 4]);
    }
}
