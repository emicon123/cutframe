//! Opening a media file with FFmpeg, cancellable through an interrupt flag (design §5).

use std::ffi::{CStr, CString, c_int, c_void};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};

use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::ffi;

use super::error::{MediaError, MediaErrorKind};

/// An open demuxer whose blocking I/O stops as soon as `cancel` is set.
///
/// The interrupt callback points at `cancel`, so the context must not outlive it: the lifetime
/// `'c` ties them together.
pub(crate) struct Input<'c> {
    ctx: AVFormatContextInput,
    _cancel: PhantomData<&'c AtomicBool>,
}

impl std::fmt::Debug for Input<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Input").finish_non_exhaustive()
    }
}

/// FFmpeg calls this during blocking operations; non-zero aborts them with `AVERROR_EXIT`.
unsafe extern "C" fn interrupt(opaque: *mut c_void) -> c_int {
    // SAFETY: `opaque` is the `&AtomicBool` given to `open`, which outlives the context (`Input`).
    let cancel = unsafe { &*opaque.cast::<AtomicBool>() };
    c_int::from(cancel.load(Ordering::Relaxed))
}

/// A UTF-8, NUL-free path for FFmpeg. FFmpeg's `file` protocol converts it to UTF-16 on Windows,
/// so non-ASCII names work.
pub(crate) fn ffmpeg_url(path: &Path) -> Result<CString, MediaError> {
    path.to_str()
        .and_then(|s| CString::new(s).ok())
        .ok_or_else(|| {
            MediaError::new(
                MediaErrorKind::UnsupportedPath,
                format!("not valid UTF-8 or contains NUL: {}", path.display()),
            )
        })
}

/// Maps an FFmpeg error code from opening or reading.
pub(crate) fn map_error(code: c_int, context: &str) -> MediaError {
    let kind = match code {
        ffi::AVERROR_EXIT => MediaErrorKind::Cancelled,
        c if c == ffi::AVERROR(ffi::ENOENT) => MediaErrorKind::FileNotFound,
        c if c == ffi::AVERROR(ffi::EACCES) => MediaErrorKind::AccessDenied,
        ffi::AVERROR_INVALIDDATA | ffi::AVERROR_DEMUXER_NOT_FOUND => {
            MediaErrorKind::UnsupportedFormat
        }
        _ => MediaErrorKind::Damaged,
    };
    MediaError::new(kind, format!("{context}: {}", error_string(code)))
}

pub(crate) fn error_string(code: c_int) -> String {
    let mut buf = [0 as std::ffi::c_char; 128];
    // SAFETY: `buf` is writable for its full length; FFmpeg always NUL-terminates it.
    unsafe { ffi::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    // SAFETY: NUL-terminated above.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

impl<'c> Input<'c> {
    /// Opens `path` and reads stream info (with FFmpeg's default, bounded probe size and
    /// analyze duration).
    pub(crate) fn open(path: &Path, cancel: &'c AtomicBool) -> Result<Self, MediaError> {
        let url = ffmpeg_url(path)?;
        // SAFETY: plain allocation; checked for NULL below.
        let mut raw = unsafe { ffi::avformat_alloc_context() };
        if raw.is_null() {
            return Err(MediaError::new(MediaErrorKind::Damaged, "out of memory"));
        }
        // SAFETY: `raw` is a freshly allocated context; `cancel` outlives it (see `Input`).
        unsafe {
            (*raw).interrupt_callback = ffi::AVIOInterruptCB {
                callback: Some(interrupt),
                opaque: ptr::from_ref(cancel).cast_mut().cast(),
            };
        }
        // SAFETY: `raw` and `url` are valid; on failure FFmpeg frees the context and sets `raw` to
        // NULL, so nothing leaks and nothing is freed twice.
        let ret = unsafe {
            ffi::avformat_open_input(&mut raw, url.as_ptr(), ptr::null(), ptr::null_mut())
        };
        if ret < 0 {
            return Err(map_error(ret, "open"));
        }
        let raw = NonNull::new(raw).expect("avformat_open_input succeeded");
        // SAFETY: an opened input context; `AVFormatContextInput` closes it on drop.
        let mut ctx = unsafe { AVFormatContextInput::from_raw(raw) };
        // SAFETY: an opened input context.
        let ret = unsafe { ffi::avformat_find_stream_info(ctx.as_mut_ptr(), ptr::null_mut()) };
        if ret < 0 {
            return Err(map_error(ret, "find stream info"));
        }
        Ok(Self {
            ctx,
            _cancel: PhantomData,
        })
    }

    pub(crate) fn ctx(&self) -> &ffi::AVFormatContext {
        &self.ctx
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::AVFormatContext {
        self.ctx.as_mut_ptr()
    }

    /// The streams of the file.
    pub(crate) fn streams(&self) -> &[&ffi::AVStream] {
        let ctx = self.ctx();
        if ctx.streams.is_null() || ctx.nb_streams == 0 {
            return &[];
        }
        // SAFETY: `streams` holds `nb_streams` valid, non-NULL stream pointers owned by the
        // context, which lives as long as `self`. `&AVStream` has the layout of `*mut AVStream`.
        unsafe {
            std::slice::from_raw_parts(
                ctx.streams.cast::<&ffi::AVStream>(),
                ctx.nb_streams as usize,
            )
        }
    }

    /// The short name of the demuxer, e.g. `mov` (the first of `mov,mp4,m4a,...`).
    pub(crate) fn format_name(&self) -> String {
        let ctx = self.ctx();
        if ctx.iformat.is_null() {
            return String::new();
        }
        // SAFETY: `iformat` is a static demuxer description with a NUL-terminated name.
        let name = unsafe { CStr::from_ptr((*ctx.iformat).name) }.to_string_lossy();
        name.split(',').next().unwrap_or_default().to_owned()
    }

    /// The next packet, `Ok(None)` at the end of the file.
    pub(crate) fn read_packet(&mut self) -> Result<Option<rsmpeg::avcodec::AVPacket>, MediaError> {
        let mut packet = rsmpeg::avcodec::AVPacket::new();
        // SAFETY: an opened input context and an allocated packet.
        let ret = unsafe { ffi::av_read_frame(self.as_mut_ptr(), packet.as_mut_ptr()) };
        match ret {
            0.. => Ok(Some(packet)),
            ffi::AVERROR_EOF => Ok(None),
            code => Err(map_error(code, "read packet")),
        }
    }
}
