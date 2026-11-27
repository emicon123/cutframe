//! The export loop (export.md §4.2–§4.4, §5.2–§5.4): one project snapshot rendered frame by
//! frame through the [`OfflineRenderer`] (exact source frames, the `Rgba16Float` canvas kept on
//! the GPU), converted to NV12 and read back through the staging ring ([`Nv12Pass`]), encoded,
//! and muxed with the AAC track into `<stem>.cutframe-partial.mp4`, which is renamed over the
//! target at the end.
//!
//! - **Stages.** `Preparing` (media check, stale partial file, renderer, encoder, AAC, muxer),
//!   `Encoding` (progress: frames handed to the encoder), `Finalizing` (drain the ring, flush
//!   both encoders, trailer with the faststart rewrite, rename).
//! - **Encoder fallback** (§4.4): an encoder that fails to open, or fails before the first video
//!   packet was muxed, is closed with the muxer, the partial file is deleted, and the export
//!   starts again from frame 0 with the next candidate. After the first packet an encoder error
//!   is [`ExportErrorKind::EncoderFailed`]. An explicitly chosen encoder never falls back.
//! - **Cancel** is checked before every frame (and while throttled); the partial file is deleted.
//! - **Throttle** (§4.7): while the preview transport plays, frames start at most every 50 ms
//!   (20 frames/s), so the preview keeps the shared queue.
//! - **Disk floor**: free space on the output volume is checked before the first frame and every
//!   2 s; below [`DISK_FLOOR`] the export fails with `DiskFull`, as on `ENOSPC`.
//! - **Files** (§5.2): every failure, the cancel and every fallback restart delete the partial
//!   file; a finished file whose rename fails is kept under the partial name (`kept_path`).
//! - **Sleep** (§4.7): `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` on the export
//!   thread for the duration of [`run_export`].
//!
//! Threading: [`run_export`] runs on the dedicated below-normal `export` thread (the caller sets
//! the priority); it owns its renderer, readers, NV12 targets and encoders on the shared device
//! and touches nothing the preview uses but the device and queue (ADR-0004 §6).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::media::MediaId;
use app_core::{Project, Time};
use rsmpeg::avcodec::{AVCodecContext, AVPacket};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::audio::AudioEncoder;
use super::encoders::{
    ComGuard, EncoderCandidate, EncoderError, EncoderKind, EncoderSettings, OpenEncoder, OptionSet,
    open_encoder,
};
use super::mux::{MuxOptions, Muxer, StreamError, tag_frame};
use super::nv12::{Nv12Error, Nv12Frame, Nv12Pass, YuvLayout, alloc_video_frame};
use super::output::{free_space, partial_path};
use super::plan::ExportPlan;
use crate::render::{
    GpuContext, OfflineOptions, OfflineRenderer, OutputFormat, RenderError, RenderedFrame,
};

/// Below this much free space on the output volume the export fails with `DiskFull`.
pub const DISK_FLOOR: u64 = 64 * 1024 * 1024;

/// How often the free space is checked during an export.
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// While the preview plays, a frame starts at most this often (20 frames/s).
const PLAYBACK_FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// How long the final rename is retried (scanners briefly lock new files).
const RENAME_RETRY: Duration = Duration::from_secs(2);

/// How long an encoder may keep answering "try again" before it counts as failed.
const ENCODER_STALL: Duration = Duration::from_secs(10);

/// The stage the observer is told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportStage {
    Preparing,
    Encoding,
    Finalizing,
}

/// What the loop reports to and asks from its caller. `src-tauri` implements it over the job
/// context; tests use a fake.
pub trait ExportObserver: Send + Sync {
    fn cancelled(&self) -> bool;
    fn stage(&self, stage: ExportStage);
    /// Frames of the whole export; progress is `frame_done` calls / this.
    fn set_total(&self, frames: u32);
    /// One more frame was handed to the encoder (never counted twice across a fallback restart).
    fn frame_done(&self);
    /// The preview transport is playing (export throttles, §4.7).
    fn playback_active(&self) -> bool;
}

/// One export, everything decided before it starts.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    /// The snapshot taken at start. A different export frame rate is applied by the loop.
    pub project: Arc<Project>,
    pub plan: ExportPlan,
    /// The encoders to try, in order (already filtered by the session's probe).
    pub candidates: Vec<EncoderCandidate>,
    /// The user picked the encoder: no fallback, and a failure is `EncoderFailed`.
    pub explicit_encoder: bool,
    /// The final path, validated (`output::validate_output_path`).
    pub output: PathBuf,
    /// Replace an existing file at `output`.
    pub overwrite: bool,
}

/// A finished export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub bytes: u64,
    pub encoder: EncoderKind,
    pub options: OptionSet,
    pub frames: u32,
    pub elapsed: Duration,
}

/// The error taxonomy of export.md §5.3; the keys the UI words its message from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportErrorKind {
    MediaOffline,
    DecodeFailed,
    NoEncoder,
    EncoderFailed,
    DiskFull,
    WriteFailed,
    Gpu,
    Internal,
}

/// Why an export failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {detail}")]
pub struct ExportFailure {
    pub kind: ExportErrorKind,
    /// The media of `MediaOffline` and `DecodeFailed`.
    pub media: Option<MediaId>,
    /// A finished file kept under its partial name because the final rename failed.
    pub kept_path: Option<PathBuf>,
    /// English, for the log only.
    pub detail: String,
}

impl ExportFailure {
    fn new(kind: ExportErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            media: None,
            kept_path: None,
            detail: detail.into(),
        }
    }

    fn media(kind: ExportErrorKind, media: MediaId, detail: impl Into<String>) -> Self {
        Self {
            media: Some(media),
            ..Self::new(kind, detail)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("the export was cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(ExportFailure),
}

impl From<ExportFailure> for ExportError {
    fn from(failure: ExportFailure) -> Self {
        Self::Failed(failure)
    }
}

/// An opened video encoder as the loop drives it. [`OpenEncoder`] is the real one; tests wrap
/// it to inject failures (export.md §11 "Fallback").
pub trait VideoEncoder {
    fn candidate(&self) -> EncoderCandidate;
    /// The opened codec context (the muxer copies its parameters).
    fn context(&self) -> &AVCodecContext;
    /// The pixel format frames are handed over in.
    fn pixel_format(&self) -> ffi::AVPixelFormat;
    fn send_frame(&mut self, frame: Option<&AVFrame>) -> Result<(), RsmpegError>;
    fn receive_packet(&mut self) -> Result<AVPacket, RsmpegError>;
}

impl VideoEncoder for OpenEncoder {
    fn candidate(&self) -> EncoderCandidate {
        self.candidate
    }

    fn context(&self) -> &AVCodecContext {
        &self.context
    }

    fn pixel_format(&self) -> ffi::AVPixelFormat {
        self.pixel_format
    }

    fn send_frame(&mut self, frame: Option<&AVFrame>) -> Result<(), RsmpegError> {
        self.context.send_frame(frame)
    }

    fn receive_packet(&mut self) -> Result<AVPacket, RsmpegError> {
        self.context.receive_packet()
    }
}

/// Opens a video encoder; [`open_encoder`] outside tests.
pub type OpenVideo =
    dyn Fn(EncoderCandidate, &EncoderSettings) -> Result<Box<dyn VideoEncoder>, EncoderError>;

/// Free space on the volume of a folder; [`free_space`] outside tests.
pub type FreeSpace = dyn Fn(&Path) -> Option<u64>;

/// Test seams of [`run_export_with`]. The default is the real export.
#[derive(Default, Clone, Copy)]
pub struct ExportHooks<'a> {
    pub open_video: Option<&'a OpenVideo>,
    pub free_space: Option<&'a FreeSpace>,
    /// Fail writes with `ENOSPC` after this many bytes (faststart is off then, `MuxOptions`).
    pub write_limit: Option<u64>,
}

impl std::fmt::Debug for ExportHooks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportHooks")
            .field("open_video", &self.open_video.is_some())
            .field("free_space", &self.free_space.is_some())
            .field("write_limit", &self.write_limit)
            .finish()
    }
}

/// Runs the export of `req` on the calling thread (export.md §4.2). Blocks until it is done,
/// failed or cancelled.
pub fn run_export(
    gpu: Arc<GpuContext>,
    req: ExportRequest,
    obs: &dyn ExportObserver,
) -> Result<ExportSummary, ExportError> {
    run_export_with(gpu, req, obs, ExportHooks::default())
}

/// [`run_export`] with test seams.
pub fn run_export_with(
    gpu: Arc<GpuContext>,
    req: ExportRequest,
    obs: &dyn ExportObserver,
    hooks: ExportHooks<'_>,
) -> Result<ExportSummary, ExportError> {
    let started = Instant::now();
    let _awake = KeepAwake::new();
    // Media Foundation encoders need COM on this thread.
    let _com = ComGuard::multithreaded();
    obs.stage(ExportStage::Preparing);

    let project = with_frame_rate(&req.project, &req.plan);
    check_media(&project)?;
    let partial = partial_path(&req.output);
    remove_partial(&partial)?;
    let folder = req
        .output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let disk = DiskCheck {
        folder,
        query: hooks.free_space.unwrap_or(&free_space),
    };
    disk.check()?;

    let mut renderer = OfflineRenderer::new(Arc::clone(&gpu), OfflineOptions::default());
    let mut job = Job {
        gpu,
        project,
        plan: req.plan,
        partial: partial.clone(),
        obs,
        disk,
        mux: MuxOptions {
            faststart: hooks.write_limit.is_none(),
            write_limit: hooks.write_limit,
        },
        counted: 0,
        stats: Stats::default(),
    };
    let open: &OpenVideo = hooks.open_video.unwrap_or(&open_real);
    let settings = EncoderSettings::for_plan(&req.plan);
    let mut last_failure = String::from("no encoder candidate");
    for &candidate in &req.candidates {
        if obs.cancelled() {
            return Err(ExportError::Cancelled);
        }
        let outcome = match open(candidate, &settings) {
            Ok(encoder) => job.attempt(&mut renderer, encoder),
            Err(err) => Err(Attempt::Fallback(err.to_string())),
        };
        match outcome {
            Ok(written) => {
                let summary = finish_file(&req, &written, started, candidate, job.plan.frames)?;
                job.stats.log(&summary);
                return Ok(summary);
            }
            Err(Attempt::Fallback(detail)) => {
                remove_partial(&partial)?;
                if req.explicit_encoder {
                    return Err(ExportFailure::new(
                        ExportErrorKind::EncoderFailed,
                        format!("{:?} failed: {detail}", candidate.kind),
                    )
                    .into());
                }
                tracing::warn!(
                    ?candidate,
                    %detail,
                    "export encoder failed before the first packet; trying the next one"
                );
                last_failure = format!("{:?}: {detail}", candidate.kind);
            }
            Err(Attempt::Stop(err)) => {
                if let Err(cleanup) = remove_partial(&partial) {
                    tracing::warn!(%cleanup, "cannot delete the partial export file");
                }
                return Err(err);
            }
        }
    }
    let kind = if req.explicit_encoder {
        ExportErrorKind::EncoderFailed
    } else {
        ExportErrorKind::NoEncoder
    };
    Err(ExportFailure::new(kind, last_failure).into())
}

fn open_real(
    candidate: EncoderCandidate,
    settings: &EncoderSettings,
) -> Result<Box<dyn VideoEncoder>, EncoderError> {
    open_encoder(candidate, settings).map(|e| Box::new(e) as Box<dyn VideoEncoder>)
}

/// The snapshot with the export frame rate (export.md §4.1): a clone only when it differs.
fn with_frame_rate(project: &Arc<Project>, plan: &ExportPlan) -> Arc<Project> {
    if project.settings.frame_rate == plan.frame_rate {
        return Arc::clone(project);
    }
    let mut copy = Project::clone(project);
    copy.settings.frame_rate = plan.frame_rate;
    Arc::new(copy)
}

/// Every media file a clip uses must be there (a file deleted after the dialog checked).
fn check_media(project: &Project) -> Result<(), ExportFailure> {
    for media in project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| c.source.media())
    {
        let Some(asset) = project.media.iter().find(|a| a.id == media) else {
            return Err(ExportFailure::new(
                ExportErrorKind::Internal,
                format!("clip media {media} is not in the project"),
            ));
        };
        if !Path::new(&asset.path.absolute).is_file() {
            return Err(ExportFailure::media(
                ExportErrorKind::MediaOffline,
                media,
                format!("{} is missing", asset.path.absolute),
            ));
        }
    }
    Ok(())
}

/// Deletes the partial file if it exists.
fn remove_partial(partial: &Path) -> Result<(), ExportFailure> {
    match std::fs::remove_file(partial) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(ExportFailure::new(
            ExportErrorKind::WriteFailed,
            format!("cannot delete {}: {err}", partial.display()),
        )),
    }
}

/// Moves the finished partial file to the target (export.md §5.2).
fn finish_file(
    req: &ExportRequest,
    written: &Path,
    started: Instant,
    candidate: EncoderCandidate,
    frames: u32,
) -> Result<ExportSummary, ExportFailure> {
    let kept = |detail: String| ExportFailure {
        kept_path: Some(written.to_path_buf()),
        ..ExportFailure::new(ExportErrorKind::WriteFailed, detail)
    };
    if !req.overwrite && req.output.exists() {
        return Err(kept(format!(
            "{} appeared during the export and may not be replaced",
            req.output.display()
        )));
    }
    let deadline = Instant::now() + RENAME_RETRY;
    loop {
        match std::fs::rename(written, &req.output) {
            Ok(()) => break,
            Err(err) if Instant::now() < deadline => {
                tracing::debug!(%err, "rename of the finished export failed; retrying");
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                return Err(kept(format!(
                    "cannot rename {} to {}: {err}",
                    written.display(),
                    req.output.display()
                )));
            }
        }
    }
    let bytes = std::fs::metadata(&req.output).map_or(0, |m| m.len());
    Ok(ExportSummary {
        path: req.output.clone(),
        bytes,
        encoder: candidate.kind,
        options: candidate.options,
        frames,
        elapsed: started.elapsed(),
    })
}

/// The free-space floor.
struct DiskCheck<'a> {
    folder: PathBuf,
    query: &'a FreeSpace,
}

impl DiskCheck<'_> {
    fn check(&self) -> Result<(), ExportFailure> {
        match (self.query)(&self.folder) {
            Some(free) if free < DISK_FLOOR => Err(ExportFailure::new(
                ExportErrorKind::DiskFull,
                format!("{free} bytes free on the output volume"),
            )),
            _ => Ok(()),
        }
    }
}

/// How one encoder attempt ended, other than with a finished file.
enum Attempt {
    /// The encoder failed before the first video packet was muxed: try the next one.
    Fallback(String),
    Stop(ExportError),
}

impl From<ExportFailure> for Attempt {
    fn from(failure: ExportFailure) -> Self {
        Self::Stop(failure.into())
    }
}

impl From<ExportError> for Attempt {
    fn from(err: ExportError) -> Self {
        Self::Stop(err)
    }
}

/// A video-encoding step failed: in the encoder, or writing its packets.
enum VideoFault {
    Encoder(String),
    Stream(StreamError),
}

fn stream_failure(err: StreamError) -> ExportFailure {
    let kind = match &err {
        StreamError::DiskFull(_) => ExportErrorKind::DiskFull,
        StreamError::WriteFailed(_) => ExportErrorKind::WriteFailed,
        StreamError::Encoder(_) => ExportErrorKind::EncoderFailed,
        StreamError::Internal(_) => ExportErrorKind::Internal,
    };
    ExportFailure::new(kind, err.to_string())
}

fn render_failure(err: RenderError) -> ExportFailure {
    match err {
        RenderError::MediaOffline(media) => {
            ExportFailure::media(ExportErrorKind::MediaOffline, media, err.to_string())
        }
        RenderError::Decode(media) => {
            ExportFailure::media(ExportErrorKind::DecodeFailed, media, err.to_string())
        }
        RenderError::Gpu => ExportFailure::new(ExportErrorKind::Gpu, err.to_string()),
    }
}

fn nv12_failure(err: Nv12Error) -> ExportFailure {
    let kind = match err {
        Nv12Error::Poll(_) | Nv12Error::Map | Nv12Error::Range(_) => ExportErrorKind::Gpu,
        Nv12Error::RingFull | Nv12Error::Frame(_) => ExportErrorKind::Internal,
    };
    ExportFailure::new(kind, err.to_string())
}

/// Time spent per step, for the timing log line (export.md §10).
#[derive(Debug, Default)]
struct Stats {
    render: Duration,
    readback: Duration,
    copy: Duration,
    encode: Duration,
    audio: Duration,
    throttled: Duration,
    restarts: u32,
}

impl Stats {
    fn log(&self, summary: &ExportSummary) {
        let frames = f64::from(summary.frames.max(1));
        let ms = |d: Duration| d.as_secs_f64() * 1000.0 / frames;
        let seconds = summary.elapsed.as_secs_f64();
        tracing::info!(
            frames = summary.frames,
            elapsed_s = format!("{seconds:.2}"),
            fps = format!("{:.1}", f64::from(summary.frames) / seconds.max(1e-6)),
            encoder = ?summary.encoder,
            options = ?summary.options,
            restarts = self.restarts,
            render_ms = format!("{:.2}", ms(self.render)),
            readback_wait_ms = format!("{:.2}", ms(self.readback)),
            copy_ms = format!("{:.2}", ms(self.copy)),
            encode_mux_ms = format!("{:.2}", ms(self.encode)),
            audio_ms = format!("{:.2}", ms(self.audio)),
            throttled_ms = format!("{:.2}", ms(self.throttled)),
            bytes = summary.bytes,
            "export finished"
        );
    }
}

/// The state shared by the attempts of one export.
struct Job<'a> {
    gpu: Arc<GpuContext>,
    project: Arc<Project>,
    plan: ExportPlan,
    partial: PathBuf,
    obs: &'a dyn ExportObserver,
    disk: DiskCheck<'a>,
    mux: MuxOptions,
    /// Frames reported through `frame_done` (a restart does not count them again).
    counted: u32,
    stats: Stats,
}

/// The per-attempt encoders and output.
struct Output {
    video: Box<dyn VideoEncoder>,
    audio: AudioEncoder,
    muxer: Muxer,
    picture: AVFrame,
}

impl Job<'_> {
    /// Encodes the whole export with `video` into the partial file. Returns the finished
    /// partial file.
    fn attempt(
        &mut self,
        renderer: &mut OfflineRenderer,
        video: Box<dyn VideoEncoder>,
    ) -> Result<PathBuf, Attempt> {
        self.obs.stage(ExportStage::Preparing);
        let plan = self.plan;
        let size = (plan.width, plan.height);
        let layout = YuvLayout::from_pixel_format(video.pixel_format()).ok_or_else(|| {
            Attempt::Fallback(format!(
                "pixel format {} is not nv12 or yuv420p",
                video.pixel_format()
            ))
        })?;
        let audio = AudioEncoder::new(Arc::clone(&self.project), plan.sample_rate, plan.samples)
            .map_err(stream_failure)?;
        let muxer = Muxer::create_with(&self.partial, video.context(), audio.context(), self.mux)
            .map_err(stream_failure)?;
        let mut picture = alloc_video_frame(size, layout).map_err(nv12_failure)?;
        tag_frame(&mut picture);
        let mut out = Output {
            video,
            audio,
            muxer,
            picture,
        };
        let mut pass = Nv12Pass::new(Arc::clone(&self.gpu), size);

        self.obs.stage(ExportStage::Encoding);
        self.obs.set_total(plan.frames);
        let mut previous_start: Option<Instant> = None;
        let mut next_disk_check = Instant::now() + DISK_CHECK_INTERVAL;
        for n in 0..plan.frames {
            if self.obs.cancelled() {
                return Err(ExportError::Cancelled.into());
            }
            self.throttle(previous_start)?;
            previous_start = Some(Instant::now());

            let t = Instant::now();
            let canvas = match renderer.render_frame(
                &self.project,
                i64::from(n),
                size,
                OutputFormat::GpuCanvas,
            ) {
                Ok(RenderedFrame::Gpu(view)) => view,
                Ok(RenderedFrame::Rgba8(_)) => {
                    return Err(ExportFailure::new(
                        ExportErrorKind::Internal,
                        "the renderer returned CPU pixels",
                    )
                    .into());
                }
                Err(err) => return Err(render_failure(err).into()),
            };
            self.stats.render += t.elapsed();
            if pass.is_full() {
                let t = Instant::now();
                let frame = pass.next_ready(true).map_err(nv12_failure)?;
                self.stats.readback += t.elapsed();
                if let Some(frame) = frame {
                    self.hand_over(frame, &mut out)?;
                }
            }
            pass.submit(canvas, u64::from(n)).map_err(nv12_failure)?;
            while let Some(frame) = pass.next_ready(false).map_err(nv12_failure)? {
                self.hand_over(frame, &mut out)?;
            }
            if Instant::now() >= next_disk_check {
                self.disk.check()?;
                next_disk_check = Instant::now() + DISK_CHECK_INTERVAL;
            }
        }

        self.obs.stage(ExportStage::Finalizing);
        loop {
            let t = Instant::now();
            let frame = pass.next_ready(true).map_err(nv12_failure)?;
            self.stats.readback += t.elapsed();
            let Some(frame) = frame else { break };
            self.hand_over(frame, &mut out)?;
        }
        let t = Instant::now();
        encode_step(&mut *out.video, &mut out.muxer, None, &mut self.stats)?;
        self.stats.encode += t.elapsed();
        let t = Instant::now();
        let Output {
            mut audio,
            mut muxer,
            ..
        } = out;
        audio
            .finish(&mut |packet| muxer.write_audio(packet))
            .map_err(stream_failure)?;
        self.stats.audio += t.elapsed();
        if self.obs.cancelled() {
            return Err(ExportError::Cancelled.into());
        }
        Ok(muxer.finish().map_err(stream_failure)?)
    }

    /// While the preview plays, waits until [`PLAYBACK_FRAME_INTERVAL`] passed since the
    /// previous frame started.
    fn throttle(&mut self, previous_start: Option<Instant>) -> Result<(), ExportError> {
        let Some(previous) = previous_start else {
            return Ok(());
        };
        let t = Instant::now();
        while self.obs.playback_active() {
            let since = previous.elapsed();
            if since >= PLAYBACK_FRAME_INTERVAL {
                break;
            }
            if self.obs.cancelled() {
                return Err(ExportError::Cancelled);
            }
            std::thread::sleep((PLAYBACK_FRAME_INTERVAL - since).min(Duration::from_millis(10)));
        }
        self.stats.throttled += t.elapsed();
        Ok(())
    }

    /// Copies a read-back frame into the encoder frame, encodes it and the audio up to its end.
    fn hand_over(&mut self, frame: Nv12Frame<'_>, out: &mut Output) -> Result<(), Attempt> {
        let k = frame.index();
        let t = Instant::now();
        frame
            .copy_to_avframe(&mut out.picture)
            .map_err(nv12_failure)?;
        drop(frame);
        self.stats.copy += t.elapsed();
        out.picture.set_pts(k as i64);
        let t = Instant::now();
        let Output {
            video,
            muxer,
            picture,
            ..
        } = out;
        encode_step(&mut **video, muxer, Some(picture), &mut self.stats)?;
        self.stats.encode += t.elapsed();

        let t = Instant::now();
        let until = Time::from_frames(k as i64 + 1, self.project.settings.frame_rate);
        let Output { audio, muxer, .. } = out;
        audio
            .encode_until(until, &mut |packet| muxer.write_audio(packet))
            .map_err(stream_failure)?;
        self.stats.audio += t.elapsed();
        if k >= u64::from(self.counted) {
            self.counted += 1;
            self.obs.frame_done();
        }
        Ok(())
    }
}

/// Sends `frame` (or the flush) and muxes every packet the encoder returns. An encoder failure
/// before the first video packet is a fallback; after it, `EncoderFailed`.
fn encode_step(
    video: &mut dyn VideoEncoder,
    muxer: &mut Muxer,
    frame: Option<&AVFrame>,
    stats: &mut Stats,
) -> Result<(), Attempt> {
    match encode_video(video, muxer, frame) {
        Ok(()) => Ok(()),
        Err(VideoFault::Stream(err)) => Err(stream_failure(err).into()),
        Err(VideoFault::Encoder(detail)) if muxer.video_packets() == 0 => {
            stats.restarts += 1;
            Err(Attempt::Fallback(detail))
        }
        Err(VideoFault::Encoder(detail)) => Err(ExportFailure::new(
            ExportErrorKind::EncoderFailed,
            format!("{:?}: {detail}", video.candidate().kind),
        )
        .into()),
    }
}

/// Sends `frame` (`None` flushes and drains to the end), writing every packet to `muxer`.
fn encode_video(
    video: &mut dyn VideoEncoder,
    muxer: &mut Muxer,
    frame: Option<&AVFrame>,
) -> Result<(), VideoFault> {
    let deadline = Instant::now() + ENCODER_STALL;
    loop {
        match video.send_frame(frame) {
            Ok(()) => break,
            Err(RsmpegError::SendFrameAgainError) if Instant::now() < deadline => {
                if !drain(video, muxer)? {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Err(err) => return Err(VideoFault::Encoder(format!("send_frame: {err}"))),
        }
    }
    if frame.is_some() {
        drain(video, muxer)?;
        return Ok(());
    }
    // Flush: receive until the encoder says it is done.
    loop {
        match video.receive_packet() {
            Ok(packet) => muxer.write_video(packet).map_err(VideoFault::Stream)?,
            Err(RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(RsmpegError::EncoderDrainError) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(err) => return Err(VideoFault::Encoder(format!("flush: {err}"))),
        }
    }
}

/// Muxes every packet that is ready. `true` if there was at least one.
fn drain(video: &mut dyn VideoEncoder, muxer: &mut Muxer) -> Result<bool, VideoFault> {
    let mut any = false;
    loop {
        match video.receive_packet() {
            Ok(packet) => {
                muxer.write_video(packet).map_err(VideoFault::Stream)?;
                any = true;
            }
            Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => {
                return Ok(any);
            }
            Err(err) => return Err(VideoFault::Encoder(format!("receive_packet: {err}"))),
        }
    }
}

/// Keeps the machine from sleeping while it lives (`SetThreadExecutionState`, export.md §4.7).
/// Thread-bound: dropped on the thread that created it (not `Send`).
struct KeepAwake {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl KeepAwake {
    fn new() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{
                ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };
            // SAFETY: plain flag call on the current thread; cleared in `drop`.
            if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) }.0 == 0 {
                tracing::debug!("SetThreadExecutionState failed; the machine may sleep");
            }
        }
        Self {
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
            // SAFETY: clears this thread's requirement set in `new`.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}
