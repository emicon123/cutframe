//! D3D12VA hardware video decode on wgpu's own D3D12 device, with zero-copy frames
//! (render-graph.md §5.4). All D3D12 and FFmpeg hwcontext interop of the engine lives here, so a
//! wgpu or FFmpeg update touches one place.
//!
//! One hardware reader works like this:
//! 1. [`HwDevice::new`] wraps wgpu's `ID3D12Device` in an FFmpeg `AVHWDeviceContext` of type
//!    `AV_HWDEVICE_TYPE_D3D12VA`, so decoder and renderer use the same device (and, on a hybrid
//!    laptop, the same GPU). One per `GpuContext`, shared by every reader.
//! 2. [`open_hw_decoder`] opens a decoder for a stream with that device. Its `get_format` picks
//!    `AV_PIX_FMT_D3D12` when the hardware supports the profile; otherwise decoding fails and
//!    [`hw_active`] tells the reader to reopen the stream in software.
//! 3. On the decode thread, [`FrameWrapper::wrap`] waits on the frame's decode fence and wraps its
//!    `ID3D12Resource` as an `NV12` or `P010` [`GpuFrame`]. If the device lacks those formats or
//!    the wrap fails, [`download`] copies the frame to CPU memory (`NV12` or `P010` planes).
//! 4. On the render thread, the frame is sampled through [`GpuFrame::plane_views`];
//!    [`GpuFrame::record_release`] goes into the same command encoder after the last use, and the
//!    frame is dropped only after that submission has completed. Dropping it returns the surface
//!    to FFmpeg's pool.
//!
//! The structs of `libavutil/hwcontext_d3d12va.h` are not in the rsmpeg bindings; they are
//! mirrored below for FFmpeg 7.1 and checked by a layout test and by runtime checks after
//! `av_hwdevice_ctx_init`.

use std::ffi::{CStr, c_void};
use std::ptr::{self, NonNull};

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_MESSAGE, D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE,
    D3D12_RESOURCE_FLAGS, ID3D12Device, ID3D12Fence, ID3D12InfoQueue, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_FORMAT_P010};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::Interface;

use super::error::{MediaError, MediaErrorKind};
use super::input::{error_string, map_error};
use super::probe::{codec_name, par};
use crate::render::GpuContext;

/// Decoded frames a reader queues ahead (render-graph.md §6).
pub(crate) const READER_QUEUE_DEPTH: usize = 6;

/// Surfaces added to the decoder's own references (render-graph.md §5.4): the reader's queue plus
/// one frame being converted and one being presented.
const EXTRA_HW_FRAMES: i32 = READER_QUEUE_DEPTH as i32 + 2;

/// Pictures narrower or lower than this are never opened in hardware. A D3D12VA decoder whose
/// hwaccel fails to initialise for an unsupported extent (a 64 x 64 HEVC stream on the RTX 3070
/// Ti, driver 32.0.16.1742) removes the whole device with `DXGI_ERROR_DRIVER_INTERNAL_ERROR`,
/// which would take the preview down with it (0320 spike). Such small pictures decode in
/// software in no time.
pub(crate) const MIN_HW_EXTENT: i32 = 256;

/// How long the decode thread waits for one frame's decode fence before giving up on the frame.
const FENCE_TIMEOUT_MS: u32 = 2_000;

// ---- FFmpeg 7.1 `libavutil/hwcontext_d3d12va.h` ----------------------------------------------

/// `AVD3D12VADeviceContext`: `AVHWDeviceContext.hwctx` for `AV_HWDEVICE_TYPE_D3D12VA`.
#[repr(C)]
struct AVD3D12VADeviceContext {
    /// `ID3D12Device *`, the one mandatory field. FFmpeg releases it with the context.
    device: *mut c_void,
    /// `ID3D12VideoDevice *`, set from `device` by `av_hwdevice_ctx_init`.
    video_device: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
}

/// `AVD3D12VASyncContext`: the fence FFmpeg signals when a frame's decode has finished.
#[repr(C)]
struct AVD3D12VASyncContext {
    /// `ID3D12Fence *`.
    fence: *mut c_void,
    /// FFmpeg's own event for this fence; not used here (see [`FrameWrapper`]).
    event: HANDLE,
    fence_value: u64,
}

/// `AVD3D12VAFrame`: what `AVFrame.data[0]` points to for `AV_PIX_FMT_D3D12`.
#[repr(C)]
struct AVD3D12VAFrame {
    /// `ID3D12Resource *`, owned by the frame's buffer reference.
    texture: *mut c_void,
    sync_ctx: AVD3D12VASyncContext,
}

/// `AVD3D12VAFramesContext`: `AVHWFramesContext.hwctx` for `AV_PIX_FMT_D3D12` frames.
#[repr(C)]
struct AVD3D12VAFramesContext {
    format: DXGI_FORMAT,
    flags: D3D12_RESOURCE_FLAGS,
}

// ---- Errors ---------------------------------------------------------------------------------

/// Why hardware decode or zero-copy is not available for a device, a stream or a frame.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HwError {
    #[error("the GPU is not a D3D12 device")]
    NotDx12,
    #[error("the software rasterizer has no hardware video decoder")]
    SoftwareAdapter,
    #[error("the D3D12 device has no video interface (ID3D12VideoDevice): {0}")]
    NoVideoDevice(windows::core::Error),
    #[error("the device lacks NV12/P010 textures")]
    MissingFormats,
    #[error("{0} failed: {1}")]
    Ffmpeg(&'static str, String),
    #[error("D3D12: {0}")]
    D3d12(#[from] windows::core::Error),
    #[error("timed out waiting for the decode fence")]
    FenceTimeout,
    #[error("the GPU device was removed")]
    DeviceRemoved,
    #[error("unusable decoder frame: {0}")]
    Frame(String),
}

// ---- Device ---------------------------------------------------------------------------------

/// An FFmpeg D3D12VA device context on wgpu's own `ID3D12Device`.
pub(crate) struct HwDevice {
    /// One reference to the `AVHWDeviceContext`; decoders take their own.
    device_ref: NonNull<ffi::AVBufferRef>,
}

// SAFETY: the context is reference counted with atomic counts, `ID3D12Device` is free-threaded,
// and FFmpeg guards the context's shared staging state with its own (recursive) lock callbacks.
unsafe impl Send for HwDevice {}
// SAFETY: as above; `&HwDevice` only hands out new references (`av_buffer_ref`).
unsafe impl Sync for HwDevice {}

impl std::fmt::Debug for HwDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HwDevice").finish_non_exhaustive()
    }
}

impl HwDevice {
    /// Creates the D3D12VA device context on `gpu`'s device. Fails on non-D3D12 backends and on
    /// the software rasterizer (WARP has no video decoder): those readers decode in software.
    pub(crate) fn new(gpu: &GpuContext) -> Result<Self, HwError> {
        let info = gpu.adapter.get_info();
        if info.backend != wgpu::Backend::Dx12 {
            return Err(HwError::NotDx12);
        }
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(HwError::SoftwareAdapter);
        }
        let raw: ID3D12Device = {
            // SAFETY: the device is a D3D12 device (checked above); the guard only lives for the
            // clone, which takes a COM reference of its own.
            let hal = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() };
            hal.ok_or(HwError::NotDx12)?.raw_device().clone()
        };
        if let Err(err) = video_device(&raw) {
            return Err(HwError::NoVideoDevice(err));
        }

        // SAFETY: plain allocation; checked for NULL.
        let device_ref = unsafe { ffi::av_hwdevice_ctx_alloc(ffi::AV_HWDEVICE_TYPE_D3D12VA) };
        let device_ref = NonNull::new(device_ref)
            .ok_or_else(|| HwError::Ffmpeg("av_hwdevice_ctx_alloc", "out of memory".into()))?;
        // SAFETY: `data` of a device reference is its `AVHWDeviceContext`, whose `hwctx` is an
        // `AVD3D12VADeviceContext` for this device type. The context takes over the COM
        // reference made by `clone` above (`into_raw`) and releases it when it is freed.
        let hwctx = unsafe {
            let ctx = device_ref.as_ref().data.cast::<ffi::AVHWDeviceContext>();
            let hwctx = (*ctx).hwctx.cast::<AVD3D12VADeviceContext>();
            (*hwctx).device = raw.into_raw();
            hwctx
        };
        let device = Self { device_ref };
        // SAFETY: an allocated, filled-in device context that has not been initialised yet.
        let ret = unsafe { ffi::av_hwdevice_ctx_init(device_ref.as_ptr()) };
        if ret < 0 {
            // `device` drops here: unref frees the context, which releases the ID3D12Device.
            return Err(HwError::Ffmpeg("av_hwdevice_ctx_init", error_string(ret)));
        }
        // Layout check of the mirrored struct: init fills these from `device`.
        // SAFETY: `hwctx` stays valid while `device` holds its reference.
        let (video_device, lock) = unsafe { ((*hwctx).video_device, (*hwctx).lock) };
        if video_device.is_null() || lock.is_none() {
            return Err(HwError::Ffmpeg(
                "av_hwdevice_ctx_init",
                "unexpected AVD3D12VADeviceContext layout".into(),
            ));
        }
        tracing::info!(adapter = %info.name, "D3D12VA device ready on the render device");
        Ok(device)
    }

    fn as_ptr(&self) -> *mut ffi::AVBufferRef {
        self.device_ref.as_ptr()
    }
}

/// Severity and text of one D3D12 debug-layer message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "diagnostics used by the hardware-decode spike test"
)]
pub(crate) struct DebugMessage {
    /// `D3D12_MESSAGE_SEVERITY`: 0 corruption, 1 error, 2 warning, 3 info, 4 message.
    pub severity: i32,
    pub text: String,
}

/// Takes the messages the D3D12 debug layer stored for `gpu`'s device since the last call (also
/// those from FFmpeg's decode queue, which uses the same device). `None` when the debug layer is
/// off (`WGPU_VALIDATION` unset). Diagnostics and tests only.
#[allow(
    dead_code,
    reason = "diagnostics used by the hardware-decode spike test"
)]
pub(crate) fn take_debug_messages(gpu: &GpuContext) -> Option<Vec<DebugMessage>> {
    // SAFETY: the guard only lives for this function.
    let hal = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }?;
    let queue: ID3D12InfoQueue = hal.raw_device().cast().ok()?;
    let mut messages = Vec::new();
    // SAFETY: a valid info queue.
    let count = unsafe { queue.GetNumStoredMessages() };
    for index in 0..count {
        let mut len = 0usize;
        // SAFETY: a size query (no message pointer) for a stored message index.
        if unsafe { queue.GetMessage(index, None, &mut len) }.is_err() || len == 0 {
            continue;
        }
        // `u64` storage keeps the `D3D12_MESSAGE` header aligned.
        let mut buf = vec![0u64; len.div_ceil(8)];
        let message = buf.as_mut_ptr().cast::<D3D12_MESSAGE>();
        // SAFETY: `buf` holds `len` writable, aligned bytes, as the size query asked for.
        if unsafe { queue.GetMessage(index, Some(message), &mut len) }.is_err() {
            continue;
        }
        // SAFETY: filled by `GetMessage`; the description points into `buf`.
        let (severity, text) = unsafe {
            let m = &*message;
            let bytes = std::slice::from_raw_parts(m.pDescription, m.DescriptionByteLength);
            (
                m.Severity.0,
                String::from_utf8_lossy(bytes)
                    .trim_end_matches('\0')
                    .to_owned(),
            )
        };
        messages.push(DebugMessage { severity, text });
    }
    // SAFETY: a valid info queue.
    unsafe { queue.ClearStoredMessages() };
    Some(messages)
}

/// Why the D3D12 device of `gpu` was removed (driver reset, hang), if it was. For logs.
#[allow(
    dead_code,
    reason = "diagnostics used by the hardware-decode spike test"
)]
pub(crate) fn device_removed_reason(gpu: &GpuContext) -> Option<String> {
    // SAFETY: the guard only lives for the call.
    let hal = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }?;
    // SAFETY: a valid device; `GetDeviceRemovedReason` has no preconditions.
    unsafe { hal.raw_device().GetDeviceRemovedReason() }
        .err()
        .map(|err| err.to_string())
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        let mut raw = self.device_ref.as_ptr();
        // SAFETY: releases the one reference this value owns.
        unsafe { ffi::av_buffer_unref(&mut raw) };
    }
}

/// `IID_ID3D12VideoDevice` (d3d12video.h). Queried here rather than through the `windows` crate's
/// large `Win32_Media_MediaFoundation` feature.
const IID_ID3D12_VIDEO_DEVICE: windows::core::GUID =
    windows::core::GUID::from_u128(0x1f052807_0b46_4acc_8a89_364f793718a4);

/// Checks that `device` has the D3D12 video interface FFmpeg's `av_hwdevice_ctx_init` needs, so
/// a driver without D3D12 video reports why instead of FFmpeg's bare `EINVAL`.
fn video_device(device: &ID3D12Device) -> windows::core::Result<()> {
    let mut video: *mut c_void = ptr::null_mut();
    // SAFETY: a valid device, a valid IID and a writable out pointer.
    unsafe { device.query(&IID_ID3D12_VIDEO_DEVICE, &mut video) }.ok()?;
    // SAFETY: `query` succeeded and returned an AddRef'd interface; release it again.
    drop(unsafe { windows::core::IUnknown::from_raw(video) });
    Ok(())
}
// ---- Decoder --------------------------------------------------------------------------------

/// Whether `codec` can decode into D3D12VA surfaces on a device context.
fn has_d3d12_config(codec: *const ffi::AVCodec) -> bool {
    (0..)
        .map_while(|index| {
            // SAFETY: `codec` is a registered codec; NULL ends the list.
            let config = unsafe { ffi::avcodec_get_hw_config(codec, index) };
            // SAFETY: a non-NULL entry is a static description.
            (!config.is_null()).then(|| unsafe { &*config })
        })
        .any(|config| {
            config.pix_fmt == ffi::AV_PIX_FMT_D3D12
                && config.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX != 0
        })
}

/// The first registered decoder for `id` that supports D3D12VA. `AVCodec::find_decoder` may
/// return a library decoder without hwaccels (e.g. `libdav1d` for AV1), so all are scanned.
fn d3d12_decoder(id: ffi::AVCodecID) -> Option<rsmpeg::avcodec::AVCodecRef<'static>> {
    let mut state = ptr::null_mut();
    loop {
        // SAFETY: iterates the static codec list with its opaque state.
        let codec = unsafe { ffi::av_codec_iterate(&mut state) };
        if codec.is_null() {
            return None;
        }
        // SAFETY: a static codec description.
        let (codec_id, name) = unsafe { ((*codec).id, (*codec).name) };
        // SAFETY: as above.
        if codec_id != id || unsafe { ffi::av_codec_is_decoder(codec) } == 0 {
            continue;
        }
        if has_d3d12_config(codec) {
            // SAFETY: a codec name is a static NUL-terminated string.
            return AVCodec::find_decoder_by_name(unsafe { CStr::from_ptr(name) });
        }
    }
}

/// Opens a decoder for `stream` that decodes into D3D12VA surfaces on `device`. Fails when no
/// decoder for the codec supports D3D12VA or the picture is smaller than [`MIN_HW_EXTENT`]; the
/// reader then decodes in software. A profile the hardware cannot decode (4:2:2, 12-bit) opens
/// but fails at the first frame with [`hw_active`] `false`; the reader then reopens the stream
/// in software.
pub(crate) fn open_hw_decoder(
    stream: &ffi::AVStream,
    device: &HwDevice,
) -> Result<AVCodecContext, MediaError> {
    let id = par(stream).codec_id;
    let (width, height) = (par(stream).width, par(stream).height);
    if width < MIN_HW_EXTENT || height < MIN_HW_EXTENT {
        return Err(MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("{width}x{height} is below the hardware decode minimum"),
        ));
    }
    let codec = d3d12_decoder(id).ok_or_else(|| {
        MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("no D3D12VA decoder for {}", codec_name(id)),
        )
    })?;
    let mut ctx = AVCodecContext::new(&codec);
    // SAFETY: `ctx` is a fresh decoder context and `codecpar` belongs to an opened stream.
    let ret = unsafe { ffi::avcodec_parameters_to_context(ctx.as_mut_ptr(), stream.codecpar) };
    if ret < 0 {
        return Err(map_error(ret, "decoder parameters"));
    }
    ctx.set_pkt_timebase(stream.time_base);
    // SAFETY: plain fields set before `avcodec_open2`; `hw_device_ctx` takes a reference of its
    // own, which the context releases when it is freed.
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).hw_device_ctx = ffi::av_buffer_ref(device.as_ptr());
        if (*raw).hw_device_ctx.is_null() {
            return Err(MediaError::new(MediaErrorKind::Damaged, "out of memory"));
        }
        (*raw).get_format = Some(get_format);
        (*raw).extra_hw_frames = EXTRA_HW_FRAMES;
        // The hardware does the work; frame threads would only add latency and surfaces.
        (*raw).thread_count = 1;
    }
    ctx.open(None).map_err(|err| {
        MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("cannot open the {} D3D12VA decoder: {err}", codec_name(id)),
        )
    })?;
    Ok(ctx)
}

/// FFmpeg's format negotiation: D3D12 surfaces when offered and their frames context can be
/// made, else `AV_PIX_FMT_NONE`, so decoding fails and the reader reopens the stream with a
/// proper (multi-threaded) software decoder. Falling back inside this decoder is not an option:
/// it runs single-threaded, and FFmpeg's native `av1` decoder has no software path at all.
/// FFmpeg calls this again without D3D12 in the list when the hwaccel fails to initialise.
unsafe extern "C" fn get_format(
    ctx: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    for index in 0.. {
        // SAFETY: FFmpeg passes a list terminated by `AV_PIX_FMT_NONE`.
        let format = unsafe { *formats.add(index) };
        if format == ffi::AV_PIX_FMT_NONE {
            break;
        }
        if format == ffi::AV_PIX_FMT_D3D12 {
            // SAFETY: `ctx` is the decoder being negotiated, with our `hw_device_ctx`.
            match unsafe { init_frames(ctx) } {
                Ok(()) => return format,
                Err(err) => {
                    tracing::warn!(%err, "no D3D12VA frames context");
                    return ffi::AV_PIX_FMT_NONE;
                }
            }
        }
    }
    tracing::debug!("D3D12VA not offered for this stream; the reader decodes in software");
    ffi::AV_PIX_FMT_NONE
}

/// Whether `decoder` (from [`open_hw_decoder`]) has negotiated D3D12VA surfaces. `false` after a
/// decode error means the hardware cannot decode this stream: reopen it in software.
pub(crate) fn hw_active(decoder: &AVCodecContext) -> bool {
    !decoder.hw_frames_ctx.is_null()
}

/// The size of the decoder surfaces: the coded size, aligned like FFmpeg's DXVA2/D3D11VA code
/// (128 for HEVC and AV1, as the HEVC DXVA spec asks, 32 for MPEG-2, else 16), and at least the
/// size FFmpeg proposed.
///
/// FFmpeg 7.1's D3D12VA `frame_params` uses the *display* size. A 4K HEVC stream with a
/// conformance window (3840 x 2160 shown, 3840 x 2176 coded, as NVENC and many cameras write
/// it) then gets surfaces 16 lines too short, and the NVIDIA driver fails the decode with
/// `DXGI_ERROR_DRIVER_INTERNAL_ERROR` (device removed). Found by the 0320 spike.
fn surface_size(codec: ffi::AVCodecID, coded: (i32, i32), proposed: (i32, i32)) -> (i32, i32) {
    let align = match codec {
        ffi::AV_CODEC_ID_HEVC | ffi::AV_CODEC_ID_AV1 => 128,
        ffi::AV_CODEC_ID_MPEG2VIDEO => 32,
        _ => 16,
    };
    let up = |v: i32| (v + align - 1) / align * align;
    (up(coded.0.max(proposed.0)), up(coded.1.max(proposed.1)))
}

/// Creates the decoder's D3D12 frames context: surfaces of [`surface_size`], shader resources
/// allowed (zero-copy samples the surfaces) and [`EXTRA_HW_FRAMES`] on top of what the decoder
/// needs.
///
/// # Safety
/// `ctx` must be a decoder context with a D3D12VA `hw_device_ctx`, inside `get_format`.
unsafe fn init_frames(ctx: *mut ffi::AVCodecContext) -> Result<(), HwError> {
    let mut frames: *mut ffi::AVBufferRef = ptr::null_mut();
    // SAFETY: guaranteed by the caller.
    let ret = unsafe {
        ffi::avcodec_get_hw_frames_parameters(
            ctx,
            (*ctx).hw_device_ctx,
            ffi::AV_PIX_FMT_D3D12,
            &mut frames,
        )
    };
    if ret < 0 {
        return Err(HwError::Ffmpeg(
            "avcodec_get_hw_frames_parameters",
            error_string(ret),
        ));
    }
    // SAFETY: `frames` is a new, uninitialised frames context of a D3D12VA device, so its
    // `hwctx` is an `AVD3D12VAFramesContext`.
    unsafe {
        let fc = (*frames).data.cast::<ffi::AVHWFramesContext>();
        let (width, height) = surface_size(
            (*ctx).codec_id,
            ((*ctx).coded_width, (*ctx).coded_height),
            ((*fc).width, (*fc).height),
        );
        (*fc).width = width;
        (*fc).height = height;
        if (*fc).initial_pool_size > 0 {
            (*fc).initial_pool_size += EXTRA_HW_FRAMES;
        }
        let hw = (*fc).hwctx.cast::<AVD3D12VAFramesContext>();
        (*hw).flags =
            D3D12_RESOURCE_FLAGS((*hw).flags.0 & !D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE.0);
        tracing::debug!(
            width = (*fc).width,
            height = (*fc).height,
            sw_format = (*fc).sw_format,
            pool = (*fc).initial_pool_size,
            dxgi_format = (*hw).format.0,
            flags = (*hw).flags.0,
            "D3D12VA frames context"
        );
    }
    // SAFETY: a filled-in frames context.
    let ret = unsafe { ffi::av_hwframe_ctx_init(frames) };
    if ret < 0 {
        // SAFETY: releases the reference made above.
        unsafe { ffi::av_buffer_unref(&mut frames) };
        return Err(HwError::Ffmpeg("av_hwframe_ctx_init", error_string(ret)));
    }
    // SAFETY: the decoder takes over our reference (and drops any earlier one).
    unsafe {
        ffi::av_buffer_unref(&mut (*ctx).hw_frames_ctx);
        (*ctx).hw_frames_ctx = frames;
    }
    Ok(())
}

/// Whether `frame` is a D3D12VA surface (else the decoder fell back to software).
pub(crate) fn is_hw_frame(frame: &AVFrame) -> bool {
    frame.format == ffi::AV_PIX_FMT_D3D12 && !frame.data[0].is_null()
}

/// The D3D12 parts of a hardware frame.
fn hw_parts(frame: &AVFrame) -> Result<&AVD3D12VAFrame, HwError> {
    if !is_hw_frame(frame) {
        return Err(HwError::Frame("not a D3D12 frame".into()));
    }
    // SAFETY: for `AV_PIX_FMT_D3D12`, `data[0]` points to an `AVD3D12VAFrame` that lives as long
    // as the frame's buffer reference, i.e. at least as long as `frame`.
    Ok(unsafe { &*frame.data[0].cast::<AVD3D12VAFrame>() })
}

/// A stable identity of the decoder surface behind `frame` (diagnostics: counting pool
/// surfaces).
#[allow(
    dead_code,
    reason = "diagnostics used by the hardware-decode spike test"
)]
pub(crate) fn surface_id(frame: &AVFrame) -> Option<usize> {
    hw_parts(frame).ok().map(|hw| hw.texture as usize)
}

/// Copies a hardware frame to CPU memory (`NV12` or `P010` planes), with its properties (pts).
/// FFmpeg orders the copy after the decode on the GPU, so no fence wait is needed first.
pub(crate) fn download(frame: &AVFrame) -> Result<AVFrame, MediaError> {
    let mut cpu = AVFrame::new();
    cpu.hwframe_transfer_data(frame).map_err(|err| {
        MediaError::new(
            MediaErrorKind::Damaged,
            format!("hardware frame download: {err}"),
        )
    })?;
    // SAFETY: two valid frames.
    let ret = unsafe { ffi::av_frame_copy_props(cpu.as_mut_ptr(), frame.as_ptr()) };
    if ret < 0 {
        return Err(map_error(ret, "frame properties"));
    }
    Ok(cpu)
}

// ---- Zero-copy frames -----------------------------------------------------------------------

/// Keeps an `AVFrame` reference, so FFmpeg does not reuse its surface.
struct HwFrameRef(AVFrame);

// SAFETY: the frame is never mutated after the wrap; shared access is limited to reading its
// fields (`GpuFrame::av_frame`), and FFmpeg's buffer references are atomically counted, so the
// one drop (by the last owner of the `GpuFrame`) may run on any thread. This lets readers share
// decoded frames with the render thread through an `Arc`.
unsafe impl Sync for HwFrameRef {}

/// A hardware-decoded frame on the shared device (render-graph.md §5.3). `Send`: moved from the
/// decode thread to the render thread.
pub struct GpuFrame {
    /// The decoder's `ID3D12Resource`, wrapped for this frame. `TextureFormat::NV12` (8-bit) or
    /// `P010` (10-bit, MSB-aligned); sampled through [`plane_views`](Self::plane_views).
    pub texture: wgpu::Texture,
    /// Size of the surface (e.g. 3840 x 2176 for 4K HEVC).
    pub coded: (u32, u32),
    /// The picture inside it (e.g. 3840 x 2160), from the top-left corner.
    pub visible: (u32, u32),
    /// Dropped only after the GPU work that sampled the texture has completed.
    hold: HwFrameRef,
}

impl std::fmt::Debug for GpuFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuFrame")
            .field("format", &self.texture.format())
            .field("coded", &self.coded)
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

impl GpuFrame {
    /// Views of the luma (`R8Unorm` / `R16Unorm`) and chroma (`Rg8Unorm` / `Rg16Unorm`) planes.
    pub fn plane_views(&self) -> [wgpu::TextureView; 2] {
        [wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1].map(|aspect| {
            self.texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("decoded plane"),
                format: self.texture.format().aspect_specific_format(aspect),
                aspect,
                ..Default::default()
            })
        })
    }

    /// Records the transition of the surface back to `D3D12_RESOURCE_STATE_COMMON`, the state
    /// FFmpeg's decoder expects when it uses the surface again (as a reference frame or as a new
    /// output). Record it into the encoder of the last use, after that use.
    pub fn record_release(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &self.texture,
                selector: None,
                // wgpu-hal dx12 maps PRESENT to D3D12_RESOURCE_STATE_COMMON.
                state: wgpu::TextureUses::PRESENT,
            }),
        );
    }

    /// The `AVFrame` behind the texture (pts, colour properties).
    #[allow(
        dead_code,
        reason = "diagnostics used by the hardware-decode spike test"
    )]
    pub(crate) fn av_frame(&self) -> &AVFrame {
        &self.hold.0
    }
}

/// Waits for decoder fences and wraps surfaces, on one reader's decode thread.
pub(crate) struct FrameWrapper {
    device: wgpu::Device,
    zero_copy: bool,
    /// Our own auto-reset event: FFmpeg's `sync_ctx.event` belongs to its own waits.
    event: HANDLE,
}

// SAFETY: the event handle is owned by this value and only used by whoever owns it.
unsafe impl Send for FrameWrapper {}

impl std::fmt::Debug for FrameWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameWrapper")
            .field("zero_copy", &self.zero_copy)
            .finish_non_exhaustive()
    }
}

impl FrameWrapper {
    pub(crate) fn new(gpu: &GpuContext) -> Result<Self, HwError> {
        // SAFETY: creates an unnamed auto-reset event; closed in `drop`.
        let event = unsafe { CreateEventW(None, false, false, None) }?;
        Ok(Self {
            device: gpu.device.clone(),
            zero_copy: gpu.supports_zero_copy_video(),
            event,
        })
    }

    /// Whether frames can be wrapped at all on this device ([`GpuContext::supports_zero_copy_video`]).
    pub(crate) fn zero_copy(&self) -> bool {
        self.zero_copy
    }

    /// Blocks until the decode of `frame` has finished on the GPU.
    pub(crate) fn wait(&self, frame: &AVFrame) -> Result<(), HwError> {
        let sync = &hw_parts(frame)?.sync_ctx;
        // SAFETY: the fence pointer of a live frame is a valid `ID3D12Fence` owned by FFmpeg;
        // borrowing it takes no reference.
        let fence = unsafe { ID3D12Fence::from_raw_borrowed(&sync.fence) }
            .ok_or_else(|| HwError::Frame("no fence".into()))?;
        // SAFETY: a valid fence.
        let completed = unsafe { fence.GetCompletedValue() };
        if completed == u64::MAX {
            return Err(HwError::DeviceRemoved);
        }
        if completed >= sync.fence_value {
            return Ok(());
        }
        // SAFETY: a valid fence and our own event handle.
        unsafe { fence.SetEventOnCompletion(sync.fence_value, self.event) }?;
        // SAFETY: our own event handle.
        if unsafe { WaitForSingleObject(self.event, FENCE_TIMEOUT_MS) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(HwError::FenceTimeout)
        }
    }

    /// Waits for `frame`'s decode and wraps its surface. On failure the frame comes back with the
    /// reason, so the caller can [`download`] it instead.
    pub(crate) fn wrap(&self, frame: AVFrame) -> Result<GpuFrame, (AVFrame, HwError)> {
        match self.try_wrap(&frame) {
            Ok((texture, coded)) => {
                let visible = (
                    u32::try_from(frame.width).unwrap_or(0).min(coded.0),
                    u32::try_from(frame.height).unwrap_or(0).min(coded.1),
                );
                Ok(GpuFrame {
                    texture,
                    coded,
                    visible,
                    hold: HwFrameRef(frame),
                })
            }
            Err(err) => Err((frame, err)),
        }
    }

    fn try_wrap(&self, frame: &AVFrame) -> Result<(wgpu::Texture, (u32, u32)), HwError> {
        if !self.zero_copy {
            return Err(HwError::MissingFormats);
        }
        self.wait(frame)?;
        let hw = hw_parts(frame)?;
        // SAFETY: the texture pointer of a live frame is a valid `ID3D12Resource`; `cloned`
        // takes a COM reference of its own, which the wgpu texture releases when dropped.
        let resource = unsafe { ID3D12Resource::from_raw_borrowed(&hw.texture) }
            .cloned()
            .ok_or_else(|| HwError::Frame("no texture".into()))?;
        // SAFETY: a valid resource.
        let desc = unsafe { resource.GetDesc() };
        let format = match desc.Format {
            DXGI_FORMAT_NV12 => wgpu::TextureFormat::NV12,
            DXGI_FORMAT_P010 => wgpu::TextureFormat::P010,
            other => return Err(HwError::Frame(format!("DXGI format {}", other.0))),
        };
        let width = u32::try_from(desc.Width).map_err(|_| HwError::Frame("width".into()))?;
        if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
            || desc.DepthOrArraySize != 1
            || desc.MipLevels != 1
            || desc.SampleDesc.Count != 1
            || width % 2 != 0
            || desc.Height % 2 != 0
        {
            return Err(HwError::Frame(format!(
                "unexpected surface {width}x{} array {} mips {}",
                desc.Height, desc.DepthOrArraySize, desc.MipLevels
            )));
        }
        if desc.Flags.0 & D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE.0 != 0 {
            return Err(HwError::Frame("surface denies shader access".into()));
        }
        let size = wgpu::Extent3d {
            width,
            height: desc.Height,
            depth_or_array_layers: 1,
        };
        // SAFETY: a 2D, single-mip, single-sample resource of exactly this format and size.
        let hal = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("decoded frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // SAFETY: the resource was created on this device (FFmpeg uses wgpu's device), matches
        // `descriptor`, holds a finished decode (fence waited above), and is in
        // D3D12_RESOURCE_STATE_COMMON, which FFmpeg's decoder leaves it in and which wgpu-hal
        // dx12 writes as PRESENT.
        let texture = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal,
                &descriptor,
                wgpu::TextureUses::PRESENT,
            )
        };
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(HwError::Frame(format!("wrap: {err}")));
        }
        Ok((texture, (width, desc.Height)))
    }
}

impl Drop for FrameWrapper {
    fn drop(&mut self) {
        // SAFETY: closes the event this value created.
        unsafe { CloseHandle(self.event) }.ok();
    }
}

#[cfg(test)]
mod spike;

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use super::*;

    /// The mirrored structs match the x64 layout of FFmpeg 7.1's `hwcontext_d3d12va.h`.
    #[test]
    fn hwcontext_structs_have_the_c_layout() {
        assert_eq!(size_of::<AVD3D12VADeviceContext>(), 40);
        assert_eq!(offset_of!(AVD3D12VADeviceContext, video_device), 8);
        assert_eq!(offset_of!(AVD3D12VADeviceContext, lock_ctx), 32);
        assert_eq!(size_of::<AVD3D12VASyncContext>(), 24);
        assert_eq!(offset_of!(AVD3D12VASyncContext, fence_value), 16);
        assert_eq!(size_of::<AVD3D12VAFrame>(), 32);
        assert_eq!(offset_of!(AVD3D12VAFrame, sync_ctx), 8);
        assert_eq!(size_of::<AVD3D12VAFramesContext>(), 8);
        assert_eq!(offset_of!(AVD3D12VAFramesContext, flags), 4);
    }

    #[test]
    fn gpu_frames_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<GpuFrame>();
        assert_send::<FrameWrapper>();
        assert_send::<HwDevice>();
    }

    #[test]
    fn surfaces_cover_the_coded_size() {
        // 4K HEVC with a conformance window: FFmpeg 7.1 proposes the display size.
        assert_eq!(
            surface_size(ffi::AV_CODEC_ID_HEVC, (3840, 2176), (3840, 2160)),
            (3840, 2176)
        );
        assert_eq!(
            surface_size(ffi::AV_CODEC_ID_HEVC, (1920, 1088), (1920, 1080)),
            (1920, 1152)
        );
        assert_eq!(
            surface_size(ffi::AV_CODEC_ID_H264, (3840, 2160), (3840, 2160)),
            (3840, 2160)
        );
        assert_eq!(
            surface_size(ffi::AV_CODEC_ID_H264, (1920, 1088), (1920, 1080)),
            (1920, 1088)
        );
        assert_eq!(
            surface_size(ffi::AV_CODEC_ID_MPEG2VIDEO, (720, 576), (720, 576)),
            (736, 576)
        );
    }

    #[test]
    fn extra_frames_cover_the_reader_queue() {
        assert_eq!(EXTRA_HW_FRAMES, 8);
    }
}
