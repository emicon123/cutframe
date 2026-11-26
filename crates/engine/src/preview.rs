//! The preview thread. It owns the preview surface and renders the preview either into a native
//! window (Windows: a child window over the UI's preview area) or into offscreen frames that the
//! UI pulls (legacy Linux `frames` mode, ADR-0005 amendment).
//!
//! All communication goes through a channel, so callers never block on rendering.
//!
//! **What it draws** (render-graph.md §4, §7, §8): the timeline frame at the playhead.
//! [`build_plan`] at the [`TransportShared`] playhead and a render size fitted into the window
//! (full quality; quality and zoom are task 0340), then the shared [`Compositor`] with
//! `FetchPolicy::BestAvailable` on the thread's own [`ReaderPool`], then a present pass that
//! scales the canvas into the `Bgra8Unorm` surface with a black letterbox. A layer whose frame is
//! missing (not stale) keeps the previous canvas for up to [`MISSING_HOLD`], then is left out.
//! Readers report offline media through the `on_media_unavailable` callback.
//!
//! **Render on change** (render-graph.md §7, ADR-0005 amendment): while paused the thread blocks
//! on its channel and draws once per wake message ([`PreviewHandle::snapshot_changed`],
//! [`PreviewHandle::seek`], transport changes, resize, becoming visible,
//! [`FrameReadyNotifier::notify`], [`PreviewHandle::request_redraw`] and device recreation). A
//! draw whose plan and frames equal the last composite presents nothing, unless the surface needs
//! a new frame (resize, shown, redraw request).
//!
//! **Playback** (playback.md §5, §7; render-graph.md §7 "Playing"): while the transport plays the
//! thread is vsync-paced. Every vsync it asks the [`AudioSink`] for the audible time, shows the
//! frame `floor((t_audible + lead x rate) / frame)` with `lead` = 2 measured refresh intervals
//! ([`FramePicker`]), composites only when that frame or the plan changed (the compositor skips
//! the rest) and always presents. It ends preroll (first frame ready and ring filled, or
//! [`PREROLL_TIMEOUT`]), detects the end of the timeline, counts dropped frames, primes readers
//! for clips that start within the lookahead ([`scheduler::lookahead`], `ReaderPool::prime`),
//! sets keyframe-only decode above 2x, and emits `playback-tick`s through the tick callback (at
//! most 60 Hz while playing, once after every change). With the preview hidden it wakes every
//! [`HIDDEN_PLAYBACK_WAKE`] for ticks and end detection and presents nothing.
//!
//! **Inputs:** the project snapshot (`Arc<ArcSwap<Project>>`, published by the shell after every
//! change) and the [`TransportShared`] atomics are read lock-free when drawing. The engine never
//! takes the document lock (ADR-0004 §2).
//!
//! **GPU:** the thread creates the [`GpuContext`] (so the adapter can present to the window,
//! ADR-0005) and shares it as `Arc<GpuContext>` ([`PreviewHandle::gpu`]). When the device is lost
//! (driver reset, TDR), the thread recreates the context on the same window following
//! [`after_device_loss`], publishes the **new** `Arc` and reports the new GPU through the
//! `on_gpu_changed` callback. Holders of the old `Arc` (readers, compositor state, an export) must
//! drop every resource created on the old device and take the new context; `Arc::ptr_eq` tells
//! whether a held context is still the current one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use app_core::media::MediaId;
use app_core::{Project, Time};
use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::media::readers::{ReaderEvent, ReaderOptions, ReaderPool};
use crate::playback::pacing::{
    FramePicker, PREROLL_TIMEOUT, RefreshEstimate, TickDue, TickThrottle, reached_end,
};
use crate::playback::scheduler::{self, Need};
use crate::playback::{
    AudioSink, EngineAudio, NoAudio, Playback, PlaybackTicks, Rate, Timeline, TransportWake,
};
use crate::render::present::PresentPass;
use crate::render::{
    AdapterMode, Compositor, FallbackReason, GpuContext, GpuError, GpuInfo, MissingLayers,
    OUTPUT_FORMAT, OffscreenFrame, Readback, build_plan, create_canvas, render_size,
};
use crate::transport::{PlayState, SeekMode, TransportShared};

/// Called on the preview thread after the GPU changed at runtime: the new report, and whether the
/// preview still renders (`false` = it gave up after repeated device losses).
pub type GpuChanged = Box<dyn Fn(GpuInfo, bool) + Send>;

/// Called from reader threads, once per media, when a media file the preview needs is missing
/// or unreadable (render-graph.md §10). The shell marks the media offline (media-import.md
/// §6.6).
pub type MediaUnavailable = Arc<dyn Fn(MediaId) + Send + Sync>;

/// How long a layer whose frame is missing keeps the previous canvas on screen (render-graph.md
/// §7), so cut points and scrubs do not flash black.
pub const MISSING_HOLD: Duration = Duration::from_millis(250);

/// While playing with the preview hidden, the render thread wakes this often for ticks and end
/// detection (playback.md §8).
pub const HIDDEN_PLAYBACK_WAKE: Duration = Duration::from_millis(16);

/// What the preview starts with: the project, the transport and the audio path it plays with,
/// and the callbacks to the shell.
pub struct PreviewSetup {
    /// The project to show; the shell stores new snapshots into it and calls
    /// [`PreviewHandle::snapshot_changed`].
    pub snapshot: Arc<ArcSwap<Project>>,
    /// The transport atomics, shared with the audio path.
    pub transport: Arc<TransportShared>,
    /// The audio path and master clock ([`NoAudio`]: no sound, a wall clock).
    pub audio: Arc<dyn AudioSink>,
    pub on_gpu_changed: GpuChanged,
    pub on_media_unavailable: MediaUnavailable,
    /// `playback-tick`s, on the render thread.
    pub on_tick: PlaybackTicks,
}

impl std::fmt::Debug for PreviewSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewSetup").finish_non_exhaustive()
    }
}

impl PreviewSetup {
    /// A preview of `snapshot` with sound: the audio engine on the default output device (or
    /// its null output without one). Falls back to [`Self::new`] when the feeder thread cannot
    /// start. Callbacks do nothing until replaced.
    pub fn with_sound(snapshot: Arc<ArcSwap<Project>>) -> Self {
        let silent = Self::new(snapshot);
        match EngineAudio::new(Arc::clone(&silent.transport), Arc::clone(&silent.snapshot)) {
            Ok(audio) => Self {
                audio: Arc::new(audio),
                ..silent
            },
            Err(err) => {
                tracing::error!(%err, "cannot start the audio engine; playback is silent");
                silent
            }
        }
    }

    /// A silent preview of `snapshot` ([`NoAudio`]) whose callbacks do nothing.
    pub fn new(snapshot: Arc<ArcSwap<Project>>) -> Self {
        let transport = Arc::new(TransportShared::new());
        let audio = Arc::new(NoAudio::new(Arc::clone(&transport), Arc::clone(&snapshot)));
        Self {
            snapshot,
            transport,
            audio,
            on_gpu_changed: Box::new(|_, _| {}),
            on_media_unavailable: Arc::new(|_| {}),
            on_tick: Box::new(|_| {}),
        }
    }
}

/// What to recreate after the `losses`-th device loss of this preview: the first gives the
/// (possibly reset) hardware another chance, the second uses the software rasterizer, and after
/// that the preview stops (`None`).
pub fn after_device_loss(losses: u32) -> Option<AdapterMode> {
    match losses {
        0 | 1 => Some(AdapterMode::Auto),
        2 => Some(AdapterMode::SoftwareOnly),
        _ => None,
    }
}

/// Raw handles of a native window to render into.
#[derive(Clone, Copy)]
struct NativeWindow {
    window: RawWindowHandle,
    display: Option<RawDisplayHandle>,
}

// SAFETY: raw handles are plain identifiers. The caller of `PreviewHandle::spawn_with_window`
// guarantees the window outlives the handle; the render thread only uses them to create a
// surface, which every wgpu backend allows from any thread.
unsafe impl Send for NativeWindow {}

enum Message {
    Resize {
        width: u32,
        height: u32,
    },
    SetVisible(bool),
    /// The shell published a new project snapshot.
    SnapshotChanged,
    /// The playhead moved ([`TransportShared`] already holds the new time).
    Seek {
        generation: u32,
        mode: SeekMode,
    },
    /// The transport's state, rate or loop changed.
    Transport,
    /// A reader decoded a frame (coalesced by [`FrameReadyNotifier`]).
    FrameReady,
    /// Draw and present once although no input changed (`--smoke-test`).
    Redraw,
    /// The device of context `generation` was lost (sent by its device-lost callback).
    DeviceLost {
        generation: u32,
    },
    RenderFrame {
        width: u32,
        height: u32,
        reply: Sender<OffscreenFrame>,
    },
    Shutdown,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Message::Resize { .. } => "Resize",
            Message::SetVisible(_) => "SetVisible",
            Message::SnapshotChanged => "SnapshotChanged",
            Message::Seek { .. } => "Seek",
            Message::Transport => "Transport",
            Message::FrameReady => "FrameReady",
            Message::Redraw => "Redraw",
            Message::DeviceLost { .. } => "DeviceLost",
            Message::RenderFrame { .. } => "RenderFrame",
            Message::Shutdown => "Shutdown",
        })
    }
}

/// Wakes the render thread because a reader has a new frame. Cheap to clone, never blocks, and
/// safe to call from any thread: calls between two draws coalesce into one wake (an
/// `AtomicBool` guards the channel send), so a reader may call it for every decoded frame.
#[derive(Debug, Clone)]
pub struct FrameReadyNotifier {
    /// Set by [`Self::notify`], cleared by the render thread before it draws.
    pending: Arc<AtomicBool>,
    tx: Sender<Message>,
}

impl FrameReadyNotifier {
    fn new(tx: Sender<Message>) -> Self {
        Self {
            pending: Arc::new(AtomicBool::new(false)),
            tx,
        }
    }

    /// Asks for a redraw because a frame is ready. Call it after the frame is visible to the
    /// render thread (pushed to the reader's queue).
    pub fn notify(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            // A stopped render thread needs no wake.
            let _ = self.tx.send(Message::FrameReady);
        }
    }

    /// Render thread: called before drawing, so a frame that arrives during the draw wakes it
    /// again.
    fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

/// Handle to the preview thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct PreviewHandle {
    tx: Sender<Message>,
    playback: Arc<Playback>,
    frame_ready: FrameReadyNotifier,
    /// The current GPU context; replaced after a device loss, `None` once the preview gave up.
    gpu: Arc<ArcSwapOption<GpuContext>>,
    /// The current GPU; replaced by the preview thread after a device loss.
    gpu_info: Arc<ArcSwap<GpuInfo>>,
    /// False once the preview gave up after repeated device losses.
    available: Arc<AtomicBool>,
    /// Frames presented to the native window so far.
    presented: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewHandle {
    /// Starts the preview thread for offscreen rendering and initializes the GPU on it (see
    /// [`PreviewSetup`]).
    pub fn spawn(setup: PreviewSetup) -> Result<Self, GpuError> {
        Self::spawn_inner(setup, None)
    }

    /// Starts the preview thread rendering into a native window. The GPU adapter is chosen so
    /// that it can present to this window (see [`GpuContext::with_window`]). Nothing is drawn and
    /// the window is 1 x 1 pixels until [`Self::resize`] and [`Self::set_visible`] are called.
    /// [`Self::has_window`] reports whether the window could be attached.
    ///
    /// # Safety
    /// `window` (and `display`, if given) must be valid handles, and the window must stay alive
    /// until this `PreviewHandle` is dropped.
    pub unsafe fn spawn_with_window(
        setup: PreviewSetup,
        window: RawWindowHandle,
        display: Option<RawDisplayHandle>,
    ) -> Result<Self, GpuError> {
        Self::spawn_inner(setup, Some(NativeWindow { window, display }))
    }

    /// The transport over the setup's atomics; its wakes go to the render thread.
    fn new_playback(setup: &PreviewSetup, tx: &Sender<Message>) -> Arc<Playback> {
        let tx = tx.clone();
        let playback = Arc::new(Playback::new(
            Arc::clone(&setup.transport),
            Arc::clone(&setup.snapshot),
            Arc::clone(&setup.audio),
            Box::new(move |wake| {
                let message = match wake {
                    TransportWake::Changed => Message::Transport,
                    TransportWake::Seek { generation, mode } => Message::Seek { generation, mode },
                };
                // A stopped render thread needs no wake.
                let _ = tx.send(message);
            }),
        ));
        setup.audio.attach(Arc::downgrade(&playback));
        playback
    }

    fn spawn_inner(setup: PreviewSetup, window: Option<NativeWindow>) -> Result<Self, GpuError> {
        let (tx, rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let playback = Self::new_playback(&setup, &tx);
        let shared = Shared {
            presented: Arc::new(AtomicU64::new(0)),
            available: Arc::new(AtomicBool::new(true)),
            gpu: Arc::new(ArcSwapOption::empty()),
            snapshot: setup.snapshot,
            transport: setup.transport,
            playback: Arc::clone(&playback),
            frame_ready: FrameReadyNotifier::new(tx.clone()),
            tx: tx.clone(),
            on_gpu_changed: setup.on_gpu_changed,
            on_media_unavailable: setup.on_media_unavailable,
            on_tick: setup.on_tick,
        };
        let mut handle = Self {
            tx,
            playback,
            frame_ready: shared.frame_ready.clone(),
            gpu: Arc::clone(&shared.gpu),
            // Replaced by the report from the render thread below.
            gpu_info: Arc::new(ArcSwap::from_pointee(detached_info())),
            available: Arc::clone(&shared.available),
            presented: Arc::clone(&shared.presented),
            thread: None,
        };
        let thread = thread::Builder::new()
            .name("preview-render".into())
            .spawn(
                move || match create_gpu(window.as_ref(), AdapterMode::Auto) {
                    Ok((gpu, surface)) => {
                        let render = RenderLoop::new(Arc::new(gpu), surface, window, shared);
                        let _ = ready_tx.send(Ok(Arc::clone(&render.info)));
                        render.run(&rx);
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                    }
                },
            )
            .expect("failed to spawn the preview thread");
        handle.gpu_info = ready_rx
            .recv()
            .expect("preview thread exited during GPU initialization")?;
        handle.thread = Some(thread);
        Ok(handle)
    }

    /// The GPU in use now. Changes after a runtime device loss.
    pub fn gpu_info(&self) -> Arc<GpuInfo> {
        self.gpu_info.load_full()
    }

    /// The shared GPU context, for consumers on other threads (export). A new `Arc` after a
    /// device loss (see the module docs); `None` once the preview gave up.
    pub fn gpu(&self) -> Option<Arc<GpuContext>> {
        self.gpu.load_full()
    }

    /// Whether the native window passed to [`Self::spawn_with_window`] is rendered into.
    pub fn has_window(&self) -> bool {
        self.gpu_info.load().presents_to_window
    }

    /// False once the preview stopped after repeated device losses (nothing is drawn any more).
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    /// Number of frames presented to the native window so far.
    pub fn presented_frames(&self) -> u64 {
        self.presented.load(Ordering::Relaxed)
    }

    /// The transport atomics the engine threads read.
    pub fn transport(&self) -> &Arc<TransportShared> {
        self.playback.shared()
    }

    /// The transport commands (play, pause, shuttle, step, seek, loop; playback.md §8).
    pub fn playback(&self) -> &Arc<Playback> {
        &self.playback
    }

    /// A notifier for readers: wakes the render thread when a frame is ready.
    pub fn frame_ready_notifier(&self) -> FrameReadyNotifier {
        self.frame_ready.clone()
    }

    /// New size of the attached window, in physical pixels. Redraws.
    pub fn resize(&self, width: u32, height: u32) {
        self.send(Message::Resize { width, height });
    }

    /// Stops (hidden) or allows (visible) drawing into the attached window. Becoming visible
    /// redraws.
    pub fn set_visible(&self, visible: bool) {
        self.send(Message::SetVisible(visible));
    }

    /// The snapshot passed at spawn holds a new project: redraw.
    pub fn snapshot_changed(&self) {
        self.send(Message::SnapshotChanged);
    }

    /// Moves the playhead to `time`, clamped to `0..=duration` of the current snapshot, and
    /// redraws (see [`Playback::seek`]: `Exact` while playing prerolls again, `Scrub` pauses).
    /// Returns the new seek generation. Never blocks.
    pub fn seek(&self, time: app_core::Time, mode: SeekMode) -> u32 {
        self.playback.seek(time, mode)
    }

    /// Draws and presents once even though no input changed. For `--smoke-test`, which needs a
    /// number of presents from an otherwise idle preview.
    pub fn request_redraw(&self) {
        self.send(Message::Redraw);
    }

    /// Renders one offscreen frame of the timeline at the playhead (the compositor's canvas fitted
    /// into `width` x `height`, letterboxed). Returns `None` if the thread does not answer within
    /// `timeout`.
    pub fn render_frame(
        &self,
        width: u32,
        height: u32,
        timeout: Duration,
    ) -> Option<OffscreenFrame> {
        let (reply, response) = bounded(1);
        self.send(Message::RenderFrame {
            width,
            height,
            reply,
        });
        response.recv_timeout(timeout).ok()
    }

    /// Test-only: behaves as if the current device was lost.
    #[cfg(test)]
    fn simulate_device_lost(&self, generation: u32) {
        self.send(Message::DeviceLost { generation });
    }

    fn send(&self, message: Message) {
        if self.tx.send(message).is_err() {
            tracing::error!("preview thread is not running");
        }
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The report of a handle without a GPU (before the render thread answers, or detached).
fn detached_info() -> GpuInfo {
    GpuInfo {
        adapter: "none".into(),
        backend: "none".into(),
        software: false,
        device_type: crate::render::DeviceKind::Other,
        driver: String::new(),
        presents_to_window: false,
        fallback: None,
    }
}

/// A wake message seen by a [`WakeProbe`].
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    Resize {
        width: u32,
        height: u32,
    },
    SetVisible(bool),
    SnapshotChanged,
    Seek {
        generation: u32,
        mode: SeekMode,
    },
    Transport,
    FrameReady,
    Redraw,
    /// Device loss, offscreen frame request or shutdown.
    Other,
}

/// The receiving end of a [`PreviewHandle::detached`] handle: what a render thread would see.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct WakeProbe {
    rx: Receiver<Message>,
    frame_ready: FrameReadyNotifier,
}

#[cfg(any(test, feature = "test-support"))]
impl WakeProbe {
    /// The next message sent to the render thread, or `None` after `timeout`. A `FrameReady` is
    /// consumed like the render thread does (the next notify wakes again).
    pub fn next(&self, timeout: Duration) -> Option<Wake> {
        let message = self.rx.recv_timeout(timeout).ok()?;
        Some(match message {
            Message::Resize { width, height } => Wake::Resize { width, height },
            Message::SetVisible(visible) => Wake::SetVisible(visible),
            Message::SnapshotChanged => Wake::SnapshotChanged,
            Message::Seek { generation, mode } => Wake::Seek { generation, mode },
            Message::Transport => Wake::Transport,
            Message::FrameReady => {
                self.frame_ready.clear();
                Wake::FrameReady
            }
            Message::Redraw => Wake::Redraw,
            Message::DeviceLost { .. } | Message::RenderFrame { .. } | Message::Shutdown => {
                Wake::Other
            }
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PreviewHandle {
    /// A handle without a render thread or GPU, for tests of the shell: its messages go to the
    /// returned probe. [`Self::gpu`] is `None`, offscreen frames are never rendered.
    pub fn detached(snapshot: Arc<ArcSwap<Project>>) -> (Self, WakeProbe) {
        let (tx, rx) = unbounded();
        let frame_ready = FrameReadyNotifier::new(tx.clone());
        let playback = Self::new_playback(&PreviewSetup::new(snapshot), &tx);
        let handle = Self {
            tx,
            playback,
            frame_ready: frame_ready.clone(),
            gpu: Arc::new(ArcSwapOption::empty()),
            gpu_info: Arc::new(ArcSwap::from_pointee(detached_info())),
            available: Arc::new(AtomicBool::new(true)),
            presented: Arc::new(AtomicU64::new(0)),
            thread: None,
        };
        (handle, WakeProbe { rx, frame_ready })
    }
}

/// Creates a context for `window` (or offscreen) restricted to `mode`.
fn create_gpu(
    window: Option<&NativeWindow>,
    mode: AdapterMode,
) -> Result<(GpuContext, Option<wgpu::Surface<'static>>), GpuError> {
    match window {
        // SAFETY: the caller of `spawn_with_window` guarantees the handles are valid and outlive
        // the preview thread.
        Some(window) => unsafe {
            GpuContext::with_window(
                wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: window.display,
                    raw_window_handle: window.window,
                },
                mode,
            )
        },
        None => GpuContext::new_with(mode).map(|gpu| (gpu, None)),
    }
}

/// Sends [`Message::DeviceLost`] when `gpu`'s device is lost. Our own drop (`Destroyed`) is not
/// a loss. GPU errors are logged instead of panicking: after a loss, surface calls may fail until
/// the context is recreated.
fn watch_device(gpu: &GpuContext, generation: u32, tx: &Sender<Message>) {
    let tx = tx.clone();
    gpu.device.set_device_lost_callback(move |reason, message| {
        if reason == wgpu::DeviceLostReason::Destroyed {
            return;
        }
        tracing::error!(?reason, %message, "GPU device lost");
        let _ = tx.send(Message::DeviceLost { generation });
    });
    gpu.device.on_uncaptured_error(Arc::new(|err| {
        tracing::error!(%err, "GPU error in the preview");
    }));
}

/// The parts of the render loop shared with the [`PreviewHandle`].
struct Shared {
    presented: Arc<AtomicU64>,
    available: Arc<AtomicBool>,
    /// The current context, published for [`PreviewHandle::gpu`].
    gpu: Arc<ArcSwapOption<GpuContext>>,
    snapshot: Arc<ArcSwap<Project>>,
    transport: Arc<TransportShared>,
    playback: Arc<Playback>,
    /// Cleared before every draw; the readers notify it.
    frame_ready: FrameReadyNotifier,
    /// The loop's own channel, for device-lost callbacks.
    tx: Sender<Message>,
    on_gpu_changed: GpuChanged,
    on_media_unavailable: MediaUnavailable,
    on_tick: PlaybackTicks,
}

/// Surface formats the preview prefers, best first: 8-bit and **not** `*Srgb`. The working values
/// of the render graph are display-referred R'G'B' (render-graph.md §5.1), so an sRGB surface
/// would encode them a second time.
const PREFERRED_SURFACE_FORMATS: [wgpu::TextureFormat; 2] = [
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

/// The preview surface format among the `supported` ones: a preferred format, else the first
/// non-sRGB one. `Rgba16Float` is skipped: DXGI treats a float swap chain as linear scRGB.
/// `None` when every supported format is sRGB.
fn preview_surface_format(supported: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    PREFERRED_SURFACE_FORMATS
        .into_iter()
        .find(|format| supported.contains(format))
        .or_else(|| {
            supported
                .iter()
                .copied()
                .find(|format| !format.is_srgb() && *format != wgpu::TextureFormat::Rgba16Float)
        })
}

struct WindowTarget {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    present: PresentPass,
}

impl WindowTarget {
    /// Configures `surface` for presenting. `None` (logged) if the adapter cannot present to it.
    fn new(
        gpu: &GpuContext,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let Some(mut config) =
            surface.get_default_config(&gpu.adapter, width.max(1), height.max(1))
        else {
            tracing::error!("GPU adapter cannot present to the preview window");
            return None;
        };
        let capabilities = surface.get_capabilities(&gpu.adapter);
        match preview_surface_format(&capabilities.formats) {
            Some(format) => config.format = format,
            None => tracing::warn!(
                format = ?config.format, supported = ?capabilities.formats,
                "no non-sRGB preview surface format; colours will look too bright"
            ),
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&gpu.device, &config);
        tracing::info!(width, height, format = ?config.format, "preview window attached");

        let present = PresentPass::new(&gpu.device, config.format);
        Some(Self {
            surface,
            config,
            present,
        })
    }
}

/// Length of one present-rate measurement window.
const PRESENT_STATS_WINDOW: Duration = Duration::from_secs(5);
/// Intervals kept per window (preallocated once). A window that fills up is reported early, which
/// only happens above ~400 presents per second.
const PRESENT_STATS_CAPACITY: usize = 2048;
/// An interval longer than this multiple of the window's average interval counts as a long gap
/// (approximates missed vsyncs / dropped frames).
const LONG_GAP_FACTOR: f64 = 1.5;

/// Present-rate accumulator for the preview window: fed with the time of every successful
/// present, it reports statistics once per [`PRESENT_STATS_WINDOW`] of continuous drawing.
/// [`Self::reset`] drops the current window, so pauses (idle, hidden, resized, occluded) never
/// count as gaps. No allocation after construction.
struct PresentStats {
    /// Intervals between consecutive presents in the current window.
    intervals: Vec<Duration>,
    /// Sum of `intervals`.
    total: Duration,
    /// Time of the previous present, `None` right after a reset.
    last: Option<Instant>,
}

/// Statistics of one measurement window.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PresentReport {
    /// Frames presented in the window (one per measured interval).
    frames: usize,
    /// Length of the window: the sum of all intervals.
    window: Duration,
    avg_fps: f64,
    max_gap: Duration,
    /// 99th percentile interval (nearest rank).
    p99_gap: Duration,
    /// Intervals longer than [`LONG_GAP_FACTOR`] times the average interval.
    long_gaps: usize,
}

impl PresentStats {
    fn new() -> Self {
        Self {
            intervals: Vec::with_capacity(PRESENT_STATS_CAPACITY),
            total: Duration::ZERO,
            last: None,
        }
    }

    /// Drops the current window; the next present starts a new one.
    fn reset(&mut self) {
        self.intervals.clear();
        self.total = Duration::ZERO;
        self.last = None;
    }

    /// Records a present at `now`. Returns the window's statistics when it is complete; the next
    /// window then starts at `now`.
    fn record(&mut self, now: Instant) -> Option<PresentReport> {
        let last = self.last.replace(now)?;
        let interval = now.saturating_duration_since(last);
        self.intervals.push(interval);
        self.total += interval;
        if self.total < PRESENT_STATS_WINDOW && self.intervals.len() < PRESENT_STATS_CAPACITY {
            return None;
        }
        let report = self.report();
        self.intervals.clear();
        self.total = Duration::ZERO;
        Some(report)
    }

    /// Statistics of the non-empty current window. Reorders `intervals`.
    fn report(&mut self) -> PresentReport {
        let frames = self.intervals.len();
        let average = self.total.as_secs_f64() / frames as f64;
        let long_threshold = average * LONG_GAP_FACTOR;
        let long_gaps = self
            .intervals
            .iter()
            .filter(|interval| interval.as_secs_f64() > long_threshold)
            .count();
        let max_gap = self.intervals.iter().copied().max().unwrap_or_default();
        let p99_rank = (frames * 99).div_ceil(100).max(1);
        let (_, &mut p99_gap, _) = self.intervals.select_nth_unstable(p99_rank - 1);
        PresentReport {
            frames,
            window: self.total,
            avg_fps: if average > 0.0 { 1.0 / average } else { 0.0 },
            max_gap,
            p99_gap,
            long_gaps,
        }
    }
}

/// Milliseconds rounded to 0.1, for logs.
fn millis(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

/// Retry delay after a draw that could not present because the surface was busy or outdated.
const RETRY_SOON: Duration = Duration::from_millis(16);
/// Retry delay while the window is occluded (minimised) or after a validation error.
const RETRY_LATER: Duration = Duration::from_millis(100);
/// A wait for input at least this long is a pause, not a gap in the present rate.
const IDLE_RESET: Duration = Duration::from_millis(100);

/// What a handled message asks of the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handled {
    Done,
    /// An input of the picture changed: draw and present.
    Redraw,
    Shutdown,
}

/// An `Rgba16Float` canvas of one size.
struct Canvas {
    size: (u32, u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Holds a composited frame (false until the first composite into it).
    drawn: bool,
}

impl Canvas {
    /// `slot`'s canvas, recreated at `size` if it has another size.
    fn ensure<'a>(
        slot: &'a mut Option<Canvas>,
        gpu: &GpuContext,
        size: (u32, u32),
    ) -> &'a mut Canvas {
        if slot.as_ref().is_none_or(|canvas| canvas.size != size) {
            let (texture, view) = create_canvas(&gpu.device, size);
            *slot = Some(Canvas {
                size,
                _texture: texture,
                view,
                drawn: false,
            });
        }
        slot.as_mut().expect("the canvas was just created")
    }
}

/// The render graph's state on the render thread: compositor, readers and the snapshot the
/// readers know. Everything is made on one device and dropped with it.
struct Scene {
    compositor: Compositor,
    readers: ReaderPool,
    project: Option<Arc<Project>>,
    /// The project frame whose missing layer keeps the previous canvas, and since when.
    missing: Option<(i64, Instant)>,
}

/// What [`Scene::compose`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Composed {
    /// Something was recorded into the encoder: the canvas changes.
    drawn: bool,
    /// Compose again after this long (a missing layer's hold ends).
    retry: Option<Duration>,
    /// Layers shown with a stale frame or without their frame (a reader was late).
    late_layers: u32,
}

impl Scene {
    fn new(gpu: &Arc<GpuContext>, shared: &Shared) -> Self {
        let notifier = shared.frame_ready.clone();
        let unavailable = Arc::clone(&shared.on_media_unavailable);
        let readers = ReaderPool::new(
            Some(Arc::clone(gpu)),
            ReaderOptions::default(),
            move |event| match event {
                ReaderEvent::FrameReady => notifier.notify(),
                ReaderEvent::MediaUnavailable(media) => unavailable(media),
            },
        );
        Self {
            compositor: Compositor::new(gpu),
            readers,
            project: None,
            missing: None,
        }
    }

    /// Composites the frame at `time` of `project` into `slot`'s canvas, sized to fit `bounds`
    /// (render-graph.md §8, full quality). `hold` applies the missing-layer rule of §7: a layer
    /// whose frame is missing keeps the previous canvas for [`MISSING_HOLD`].
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        slot: &mut Option<Canvas>,
        project: &Arc<Project>,
        time: Time,
        bounds: (u32, u32),
        hold: bool,
    ) -> Composed {
        if self
            .project
            .as_ref()
            .is_none_or(|known| !Arc::ptr_eq(known, project))
        {
            self.readers.set_project(project);
            self.project = Some(Arc::clone(project));
        }
        let size = render_size((project.settings.width, project.settings.height), bounds, 1);
        let canvas = Canvas::ensure(slot, gpu, size);
        let plan = build_plan(project, time, size);
        let now = Instant::now();
        let since = match self.missing {
            Some((frame, since)) if frame == plan.frame => since,
            _ => now,
        };
        let keep = hold && canvas.drawn && now.duration_since(since) < MISSING_HOLD;
        let missing = if keep {
            MissingLayers::KeepCanvas
        } else {
            MissingLayers::LeaveOut
        };
        let outcome = self.compositor.render_with(
            gpu,
            encoder,
            &plan,
            &mut self.readers,
            &canvas.view,
            missing,
        );
        canvas.drawn |= outcome.drawn;
        let late_layers = outcome.stale_layers + outcome.missing_layers;
        if outcome.missing_layers == 0 {
            self.missing = None;
            return Composed {
                drawn: outcome.drawn,
                retry: None,
                late_layers,
            };
        }
        self.missing = Some((plan.frame, since));
        tracing::trace!(
            frame = plan.frame,
            missing = outcome.missing_layers,
            kept = !outcome.drawn,
            "preview frame with missing layers"
        );
        Composed {
            drawn: outcome.drawn,
            // Kept: draw again without the layer when the hold ends (a `FrameReady` may come
            // first). Left out: the reader wakes the thread when the frame arrives.
            retry: (!outcome.drawn && keep)
                .then(|| MISSING_HOLD.saturating_sub(now.duration_since(since))),
            late_layers,
        }
    }
}

/// The offscreen frames of [`PreviewHandle::render_frame`]: their own canvas and read-back
/// target, so they do not disturb the window's canvas.
struct Offscreen {
    canvas: Option<Canvas>,
    output: PresentPass,
    readback: Option<Readback>,
}

struct RenderLoop {
    gpu: Arc<GpuContext>,
    /// Adapter of `gpu`, cached for the present-rate log.
    adapter: wgpu::AdapterInfo,
    /// Increases with every recreated context; stale device-lost messages are ignored.
    generation: u32,
    /// Device losses so far (see [`after_device_loss`]).
    losses: u32,
    /// The report published to the handle.
    info: Arc<ArcSwap<GpuInfo>>,
    /// The native window, kept to recreate the surface after a device loss.
    native: Option<NativeWindow>,
    /// Last size from [`Message::Resize`], in physical pixels.
    size: (u32, u32),
    window: Option<WindowTarget>,
    visible: bool,
    /// Created on the first draw, dropped with the device.
    scene: Option<Scene>,
    /// The canvas presented into the window.
    canvas: Option<Canvas>,
    offscreen: Option<Offscreen>,
    /// The surface needs a present even if the canvas does not change (resized, shown, redraw
    /// requested, recreated).
    force_present: bool,
    shared: Shared,
    present_stats: PresentStats,
    /// The playback run on screen; `None` while paused.
    run: Option<PlayRun>,
    refresh: RefreshEstimate,
    ticks: TickThrottle,
    /// Scratch for the scheduler's lookahead.
    needs: Vec<Need>,
}

/// One playback run: from a play start (or a preroll after an exact seek) to the next
/// generation or pause.
#[derive(Debug)]
struct PlayRun {
    seek_gen: u32,
    /// When preroll started.
    since: Instant,
    picker: FramePicker,
    /// The last frame composited in this run, its loop epoch, and the rate then.
    last: Option<(i64, u32)>,
    /// Frames skipped at 1x plus frames shown late (stale or missing layers), since the play
    /// start (playback.md §5.4).
    dropped: u32,
    /// The same split up, in the current present-rate window.
    window: PlayWindow,
}

/// Playback numbers of one present-rate window (playback.md §11).
#[derive(Debug, Default, Clone, Copy)]
struct PlayWindow {
    skipped: u32,
    late: u32,
    /// Largest |shown frame start - t_show| at present.
    av_offset_max: Time,
}

impl PlayRun {
    fn new(seek_gen: u32, dropped: u32) -> Self {
        Self {
            seek_gen,
            since: Instant::now(),
            picker: FramePicker::default(),
            last: None,
            dropped,
            window: PlayWindow::default(),
        }
    }
}

/// What one draw into the window did.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    /// Draw again after this long: nothing could be presented, or a missing layer keeps the
    /// previous canvas.
    retry: Option<Duration>,
    /// A frame was presented.
    presented: bool,
    /// Layers of this composite that were late.
    late_layers: u32,
}

impl RenderLoop {
    fn new(
        gpu: Arc<GpuContext>,
        surface: Option<wgpu::Surface<'static>>,
        native: Option<NativeWindow>,
        shared: Shared,
    ) -> Self {
        let window = surface.and_then(|surface| WindowTarget::new(&gpu, surface, 1, 1));
        watch_device(&gpu, 0, &shared.tx);
        shared.gpu.store(Some(Arc::clone(&gpu)));
        let info = Arc::new(ArcSwap::from_pointee(report(&gpu, window.is_some())));
        Self {
            adapter: gpu.adapter.get_info(),
            gpu,
            generation: 0,
            losses: 0,
            info,
            native,
            size: (1, 1),
            window,
            // Hidden until the shell shows the window.
            visible: false,
            scene: None,
            canvas: None,
            offscreen: None,
            force_present: true,
            shared,
            present_stats: PresentStats::new(),
            run: None,
            refresh: RefreshEstimate::default(),
            ticks: TickThrottle::default(),
            needs: Vec::new(),
        }
    }

    /// Recreates the context after a device loss, following [`after_device_loss`], and reports
    /// the result through `on_gpu_changed`. Gives up (stops drawing) when the policy says so.
    fn recover(&mut self) {
        // Everything made on the lost device goes (render-graph.md §10): readers, compositor
        // with its source cache, canvases. A window takes one swap chain at a time: drop the old
        // surface before creating a new one.
        self.window = None;
        self.scene = None;
        self.canvas = None;
        self.offscreen = None;
        self.force_present = true;
        self.present_stats.reset();
        loop {
            self.losses += 1;
            let Some(mode) = after_device_loss(self.losses) else {
                tracing::error!(
                    losses = self.losses,
                    "GPU lost repeatedly; the preview is unavailable"
                );
                self.shared.available.store(false, Ordering::Relaxed);
                self.shared.gpu.store(None);
                let mut info = GpuInfo::clone(&self.info.load());
                info.presents_to_window = false;
                (self.shared.on_gpu_changed)(info, false);
                return;
            };
            match create_gpu(self.native.as_ref(), mode) {
                Ok((mut gpu, surface)) => {
                    gpu.fallback = Some(FallbackReason::DeviceLost);
                    self.generation += 1;
                    watch_device(&gpu, self.generation, &self.shared.tx);
                    // The loop's reference to the lost context is dropped here; other holders
                    // replace theirs when they see the new `Arc`.
                    self.gpu = Arc::new(gpu);
                    self.shared.gpu.store(Some(Arc::clone(&self.gpu)));
                    self.adapter = self.gpu.adapter.get_info();
                    let (width, height) = self.size;
                    self.window = surface
                        .and_then(|surface| WindowTarget::new(&self.gpu, surface, width, height));
                    let info = report(&self.gpu, self.window.is_some());
                    tracing::warn!(
                        adapter = %info.adapter, ?mode, losses = self.losses,
                        "GPU context recreated after a device loss"
                    );
                    self.info.store(Arc::new(info.clone()));
                    (self.shared.on_gpu_changed)(info, true);
                    return;
                }
                Err(err) => {
                    tracing::error!(%err, ?mode, "cannot recreate the GPU context");
                }
            }
        }
    }

    fn gave_up(&self) -> bool {
        !self.shared.available.load(Ordering::Relaxed)
    }

    /// Render on change while paused: block until a message arrives (or a pending tick is
    /// due), handle every queued message, then draw once if any of them changed the picture. A
    /// draw that could not present, or that keeps the canvas for a missing layer, is retried
    /// after a short delay (or with the next message). While playing: one frame per vsync
    /// (the surface acquire paces the loop), or a wake every [`HIDDEN_PLAYBACK_WAKE`] with the
    /// preview hidden.
    fn run(mut self, rx: &Receiver<Message>) {
        // A frame was asked for and is not on screen yet.
        let mut dirty = false;
        let mut retry: Option<Duration> = None;
        loop {
            let playing = self.shared.transport.is_playing();
            let wait = if playing {
                match retry {
                    Some(delay) => Some(delay.min(HIDDEN_PLAYBACK_WAKE)),
                    None if self.is_drawing() => Some(Duration::ZERO),
                    None => Some(HIDDEN_PLAYBACK_WAKE),
                }
            } else {
                retry
            };
            let wait = match self.ticks.poll(Instant::now(), playing) {
                TickDue::Now => Some(Duration::ZERO),
                TickDue::After(due) => Some(wait.map_or(due, |wait| wait.min(due))),
                TickDue::Idle => wait,
            };
            retry = None;
            let first = match wait {
                Some(delay) if delay.is_zero() => match rx.try_recv() {
                    Ok(message) => Some(message),
                    Err(crossbeam_channel::TryRecvError::Empty) => None,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => return,
                },
                Some(delay) => match rx.recv_timeout(delay) {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => {
                    // Idle: no timer, no presents until an input changes.
                    let idle = Instant::now();
                    let Ok(message) = rx.recv() else {
                        return;
                    };
                    if idle.elapsed() >= IDLE_RESET {
                        self.present_stats.reset();
                    }
                    Some(message)
                }
            };
            for message in first.into_iter().chain(rx.try_iter()) {
                match self.handle(message) {
                    Handled::Done => {}
                    Handled::Redraw => dirty = true,
                    Handled::Shutdown => return,
                }
            }
            if self.shared.transport.is_playing() && !self.gave_up() {
                retry = self.play_step();
                dirty = false;
            } else {
                if self.run.is_some() {
                    self.leave_playback();
                    dirty = true;
                }
                if !self.is_drawing() {
                    // Becoming visible or a new size redraws anyway.
                    dirty = false;
                } else if dirty {
                    let time = self.shared.transport.playhead();
                    retry = self.draw_window(time, false).retry;
                    dirty = retry.is_some();
                }
            }
            self.tick_if_due();
        }
    }

    /// Emits a `playback-tick` when the throttle allows one.
    fn tick_if_due(&mut self) {
        let now = Instant::now();
        let playing = self.shared.transport.is_playing();
        if self.ticks.poll(now, playing) == TickDue::Now {
            self.ticks.emitted(now);
            (self.shared.on_tick)(self.shared.playback.status());
        }
    }

    /// Back to render on change: readers stop decoding ahead, the refresh measurement pauses.
    fn leave_playback(&mut self) {
        self.run = None;
        self.refresh.pause();
        self.present_stats.reset();
        if let Some(scene) = &mut self.scene {
            scene.readers.set_playing(false);
            scene.readers.set_keyframes_only(false);
        }
    }

    /// One playback step (playback.md §5.2, §5.3): preroll, frame choice, end detection,
    /// composite + present (when visible), dropped-frame counting and the lookahead. Returns a
    /// delay when the surface could not present.
    fn play_step(&mut self) -> Option<Duration> {
        let transport = Arc::clone(&self.shared.transport);
        let playback = Arc::clone(&self.shared.playback);
        let seek_gen = transport.seek_generation();
        let rate = Rate::new(transport.rate())?;
        if self.run.as_ref().is_none_or(|run| run.seek_gen != seek_gen) {
            // A new play start or an exact seek while playing: dropped frames count from the
            // play start (the transport resets them).
            let dropped = playback.status().dropped_frames;
            self.run = Some(PlayRun::new(seek_gen, dropped));
            self.present_stats.reset();
        }
        let project = self.shared.snapshot.load_full();
        let timeline = Timeline::of(&project);
        let looping = transport.looping();
        let drawing = self.is_drawing();
        if drawing {
            let scene = self.scene();
            scene.readers.set_playing(rate.is_forward());
            scene
                .readers
                .set_keyframes_only(rate.get().unsigned_abs() > 2);
        }

        let state = transport.state();
        let audible = playback
            .audio()
            .audible()
            .filter(|audible| audible.seek_gen == seek_gen);
        let (time, frame, epoch, t_show) = match (state, audible) {
            (PlayState::Playing, Some(audible)) => {
                if reached_end(audible, rate, timeline, looping) {
                    playback.end(seek_gen);
                    return None;
                }
                let lead = self.refresh.lead();
                let run = self.run.as_mut().expect("a run");
                let pick = run.picker.pick(audible, lead, rate, timeline, looping);
                (
                    pick.time,
                    Some(pick.frame),
                    audible.epoch,
                    Some(pick.t_show),
                )
            }
            // Preroll, or started and waiting for the first sample to reach the device: the
            // first frame. A clock without a reading later in the run (e.g. a rate change
            // restarting it) holds the frame on screen.
            _ => {
                let held = self.run.as_ref().is_some_and(|run| run.last.is_some());
                let time = if held {
                    transport.shown()
                } else {
                    transport.playhead()
                };
                (time, None, 0, None)
            }
        };

        let mut drawn = None;
        if drawing {
            let result = self.draw_window(time, true);
            if result.presented {
                self.refresh.presented(Instant::now());
            }
            drawn = Some(result);
        }
        if transport.seek_generation() == seek_gen {
            transport.set_shown(timeline.frame_start(time));
        }

        if state == PlayState::Prerolling {
            let run = self.run.as_ref().expect("a run");
            let first_frame = drawn.is_none_or(|d| d.late_layers == 0);
            if (first_frame && playback.audio().ring_ready())
                || run.since.elapsed() >= PREROLL_TIMEOUT
            {
                tracing::debug!(
                    seek_gen,
                    preroll_ms = millis(run.since.elapsed()),
                    first_frame,
                    "preroll done"
                );
                playback.preroll_done(seek_gen);
            }
        }

        let frame_changed = {
            let run = self.run.as_mut().expect("a run");
            let key = frame.map(|frame| (frame, epoch));
            let changed = key.is_some() && run.last != key;
            if let (Some((frame, epoch)), true) = (key, changed) {
                if let Some((last, last_epoch)) = run.last
                    && rate.get() == 1
                    && last_epoch == epoch
                    && frame > last + 1
                {
                    let skipped = u32::try_from(frame - last - 1).unwrap_or(u32::MAX);
                    run.window.skipped += skipped;
                    run.dropped = run.dropped.saturating_add(skipped);
                }
                if drawn.is_some_and(|d| d.late_layers > 0)
                    && rate.get().unsigned_abs() <= 2
                    && rate.is_forward()
                {
                    run.window.late += 1;
                    run.dropped = run.dropped.saturating_add(1);
                }
                run.last = Some((frame, epoch));
                playback.set_dropped(run.dropped);
            }
            // Past the end the frame is clamped to the last one: not an A/V offset.
            if let Some(t_show) = t_show.filter(|&t| t < timeline.duration) {
                let offset = Time((time - t_show).flicks().abs());
                run.window.av_offset_max = run.window.av_offset_max.max(offset);
            }
            changed || run.last.is_none()
        };

        if drawing && frame_changed {
            scheduler::lookahead(&project, time, rate, looping, &mut self.needs);
            if let Some(scene) = &mut self.scene {
                for need in &self.needs {
                    scene.readers.prime(need.media, need.sample_time);
                }
            }
        }
        drawn.and_then(|d| d.retry.filter(|_| !d.presented))
    }

    fn is_drawing(&self) -> bool {
        self.visible
            && !self.gave_up()
            && self
                .window
                .as_ref()
                .is_some_and(|w| w.config.width > 0 && w.config.height > 0)
    }

    fn handle(&mut self, message: Message) -> Handled {
        match message {
            Message::Resize { width, height } => {
                self.size = (width.max(1), height.max(1));
                if let Some(target) = &mut self.window {
                    target.config.width = width.max(1);
                    target.config.height = height.max(1);
                    target.surface.configure(&self.gpu.device, &target.config);
                }
                self.force_present = true;
                self.present_stats.reset();
                Handled::Redraw
            }
            Message::SetVisible(visible) => {
                self.visible = visible;
                self.present_stats.reset();
                if visible {
                    self.force_present = true;
                    Handled::Redraw
                } else {
                    Handled::Done
                }
            }
            Message::SnapshotChanged => Handled::Redraw,
            Message::Redraw => {
                self.force_present = true;
                Handled::Redraw
            }
            Message::Seek { generation, mode } => {
                tracing::trace!(generation, ?mode, "seek");
                self.ticks.request();
                Handled::Redraw
            }
            Message::Transport => {
                self.ticks.request();
                Handled::Redraw
            }
            Message::FrameReady => {
                self.shared.frame_ready.clear();
                Handled::Redraw
            }
            Message::DeviceLost { generation } => {
                if generation == self.generation && !self.gave_up() {
                    self.recover();
                    Handled::Redraw
                } else {
                    Handled::Done
                }
            }
            Message::RenderFrame {
                width,
                height,
                reply,
            } => {
                if self.gave_up() {
                    // Dropping `reply` answers "no frame" right away.
                    return Handled::Done;
                }
                if let Some(frame) = self.render_offscreen(width.max(1), height.max(1)) {
                    let _ = reply.send(frame);
                }
                Handled::Done
            }
            Message::Shutdown => Handled::Shutdown,
        }
    }

    /// The scene, created on first use.
    fn scene(&mut self) -> &mut Scene {
        let (gpu, shared) = (&self.gpu, &self.shared);
        self.scene.get_or_insert_with(|| Scene::new(gpu, shared))
    }

    /// The frame at the playhead, letterboxed into `width` x `height` and read back.
    fn render_offscreen(&mut self, width: u32, height: u32) -> Option<OffscreenFrame> {
        let project = self.shared.snapshot.load_full();
        let time = self.shared.transport.playhead();
        let gpu = Arc::clone(&self.gpu);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview.offscreen"),
            });
        let offscreen = self.offscreen.get_or_insert_with(|| Offscreen {
            canvas: None,
            output: PresentPass::new(&gpu.device, OUTPUT_FORMAT),
            readback: None,
        });
        let mut canvas = offscreen.canvas.take();
        let scene = {
            let (gpu, shared) = (&self.gpu, &self.shared);
            self.scene.get_or_insert_with(|| Scene::new(gpu, shared))
        };
        scene.compose(
            &gpu,
            &mut encoder,
            &mut canvas,
            &project,
            time,
            (width, height),
            false,
        );
        let offscreen = self.offscreen.as_mut()?;
        offscreen.canvas = canvas;
        let canvas = offscreen.canvas.as_ref()?;
        if offscreen
            .readback
            .as_ref()
            .is_none_or(|r| r.size != (width, height))
        {
            offscreen.readback = Some(Readback::new(&gpu.device, (width, height)));
        }
        let readback = offscreen.readback.as_mut()?;
        offscreen.output.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &canvas.view,
            canvas.size,
            &readback.view,
            (width, height),
        );
        readback.encode_copy(&mut encoder);
        let submission = gpu.queue.submit([encoder.finish()]);
        if let Some(scene) = &mut self.scene {
            scene.compositor.submitted(&gpu);
        }
        let rgba = match readback.read(&gpu, submission) {
            Ok(rgba) => rgba.to_vec(),
            Err(err) => {
                tracing::error!(%err, "offscreen preview frame failed");
                return None;
            }
        };
        if let Some(scene) = &mut self.scene {
            scene.compositor.release_completed();
        }
        Some(OffscreenFrame {
            width,
            height,
            rgba,
        })
    }

    /// Composites the frame at `time` and presents it: when the canvas changed, or always with
    /// `always` (playback presents every vsync) or a pending forced present (resize, shown,
    /// redraw request).
    fn draw_window(&mut self, time: Time, always: bool) -> Drawn {
        let nothing = Drawn {
            retry: None,
            presented: false,
            late_layers: 0,
        };
        let Some(surface_size) = self
            .window
            .as_ref()
            .map(|target| (target.config.width, target.config.height))
        else {
            return nothing;
        };
        let project = self.shared.snapshot.load_full();
        let gpu = Arc::clone(&self.gpu);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview"),
            });
        let mut canvas = self.canvas.take();
        let composed = self.scene().compose(
            &gpu,
            &mut encoder,
            &mut canvas,
            &project,
            time,
            surface_size,
            true,
        );
        self.canvas = canvas;
        let late_layers = composed.late_layers;
        if !composed.drawn && !self.force_present && !always {
            // Same plan, same frames, same surface: nothing to present (render on change).
            return Drawn {
                retry: composed.retry,
                presented: false,
                late_layers,
            };
        }

        let Some(target) = self.window.as_mut() else {
            return nothing;
        };
        let acquired = match target.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Ok((frame, false)),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok((frame, true)),
            wgpu::CurrentSurfaceTexture::Timeout => Err(RETRY_SOON),
            wgpu::CurrentSurfaceTexture::Occluded => Err(RETRY_LATER),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                target.surface.configure(&gpu.device, &target.config);
                Err(RETRY_SOON)
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!("validation error while acquiring the preview frame");
                Err(RETRY_LATER)
            }
        };
        let (frame, suboptimal) = match acquired {
            Ok(acquired) => acquired,
            Err(delay) => {
                self.present_stats.reset();
                self.refresh.pause();
                // The composite still lands on the canvas; the retry only presents it.
                if composed.drawn {
                    gpu.queue.submit([encoder.finish()]);
                    if let Some(scene) = &mut self.scene {
                        scene.compositor.submitted(&gpu);
                    }
                }
                self.force_present = true;
                return Drawn {
                    retry: Some(delay),
                    presented: false,
                    late_layers,
                };
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        if let Some(canvas) = &self.canvas {
            target.present.encode(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &canvas.view,
                canvas.size,
                &view,
                surface_size,
            );
        }
        gpu.queue.submit([encoder.finish()]);
        if let Some(scene) = &mut self.scene {
            scene.compositor.submitted(&gpu);
        }
        gpu.queue.present(frame);
        self.force_present = false;
        self.shared.presented.fetch_add(1, Ordering::Relaxed);
        if let Some(report) = self.present_stats.record(Instant::now()) {
            let play = self
                .run
                .as_mut()
                .map(|run| std::mem::take(&mut run.window))
                .unwrap_or_default();
            let audio = self.shared.playback.audio().stats();
            tracing::debug!(
                frames = report.frames,
                window_ms = millis(report.window),
                avg_fps = (report.avg_fps * 10.0).round() / 10.0,
                max_gap_ms = millis(report.max_gap),
                p99_gap_ms = millis(report.p99_gap),
                long_gaps = report.long_gaps,
                playing = self.run.is_some(),
                dropped = play.skipped + play.late,
                skipped = play.skipped,
                late = play.late,
                av_offset_ms = (play.av_offset_max.as_seconds_f64() * 10_000.0).round() / 10.0,
                underruns = audio.underruns,
                ring_min_fill = audio.ring_min_fill,
                refresh_ms = millis(self.refresh.interval()),
                width = target.config.width,
                height = target.config.height,
                present_mode = ?target.config.present_mode,
                adapter = %self.adapter.name,
                kind = ?self.adapter.device_type,
                "preview present rate"
            );
        }

        if suboptimal {
            target.surface.configure(&gpu.device, &target.config);
            self.present_stats.reset();
        }
        Drawn {
            retry: composed.retry,
            presented: true,
            late_layers,
        }
    }
}

/// The report for `gpu`; `presents` = it renders into the native window.
fn report(gpu: &GpuContext, presents: bool) -> GpuInfo {
    GpuInfo {
        presents_to_window: presents,
        ..gpu.info()
    }
}

#[cfg(test)]
mod tests {
    use app_core::Time;

    use super::*;

    fn snapshot(project: Project) -> Arc<ArcSwap<Project>> {
        Arc::new(ArcSwap::from_pointee(project))
    }

    fn empty_project() -> Arc<ArcSwap<Project>> {
        snapshot(Project::new("Test"))
    }

    /// A project whose timeline is `seconds` long (one colour clip).
    fn project_of(seconds: i64) -> Project {
        let mut project = Project::new("Test");
        project.tracks[0].clips.push(app_core::Clip::color(
            "Colour",
            Time::ZERO,
            Time::from_seconds(seconds),
            app_core::Rgba8::BLACK,
        ));
        project
    }

    const SHORT: Duration = Duration::from_millis(50);

    #[test]
    fn surface_format_is_never_srgb() {
        use wgpu::TextureFormat::*;
        // The usual D3D12 list.
        assert_eq!(
            preview_surface_format(&[Bgra8UnormSrgb, Bgra8Unorm, Rgba8UnormSrgb, Rgba8Unorm]),
            Some(Bgra8Unorm)
        );
        assert_eq!(
            preview_surface_format(&[Rgba8UnormSrgb, Rgba8Unorm]),
            Some(Rgba8Unorm)
        );
        // No preferred format: the first non-sRGB one, never a linear float swap chain.
        assert_eq!(
            preview_surface_format(&[Bgra8UnormSrgb, Rgba16Float, Rgb10a2Unorm]),
            Some(Rgb10a2Unorm)
        );
        assert_eq!(preview_surface_format(&[Bgra8UnormSrgb]), None);
        assert_eq!(preview_surface_format(&[]), None);
    }

    #[test]
    fn seek_clamps_to_the_timeline_and_counts_generations() {
        let (preview, probe) = PreviewHandle::detached(snapshot(project_of(10)));
        assert_eq!(preview.transport().seek_generation(), 0);

        assert_eq!(preview.seek(Time::from_seconds(4), SeekMode::Scrub), 1);
        assert_eq!(preview.transport().playhead(), Time::from_seconds(4));
        assert_eq!(
            probe.next(SHORT),
            Some(Wake::Seek {
                generation: 1,
                mode: SeekMode::Scrub
            })
        );

        assert_eq!(preview.seek(Time::from_seconds(60), SeekMode::Exact), 2);
        assert_eq!(preview.transport().playhead(), Time::from_seconds(10));
        assert_eq!(preview.seek(Time(-5), SeekMode::Exact), 3);
        assert_eq!(preview.transport().playhead(), Time::ZERO);
        assert_eq!(preview.transport().seek_generation(), 3);
    }

    #[test]
    fn frame_ready_wakes_coalesce_until_the_render_thread_takes_them() {
        let (preview, probe) = PreviewHandle::detached(empty_project());
        let notifier = preview.frame_ready_notifier();
        let other = notifier.clone();
        notifier.notify();
        other.notify();
        notifier.notify();
        assert_eq!(probe.next(SHORT), Some(Wake::FrameReady));
        assert_eq!(probe.next(SHORT), None, "three notifies, one wake");

        // Taken: the next frame wakes again.
        other.notify();
        assert_eq!(probe.next(SHORT), Some(Wake::FrameReady));
    }

    #[test]
    fn inputs_send_wake_messages() {
        let (preview, probe) = PreviewHandle::detached(empty_project());
        preview.snapshot_changed();
        preview.resize(640, 360);
        preview.set_visible(true);
        preview.request_redraw();
        assert_eq!(probe.next(SHORT), Some(Wake::SnapshotChanged));
        assert_eq!(
            probe.next(SHORT),
            Some(Wake::Resize {
                width: 640,
                height: 360
            })
        );
        assert_eq!(probe.next(SHORT), Some(Wake::SetVisible(true)));
        assert_eq!(probe.next(SHORT), Some(Wake::Redraw));
        assert_eq!(probe.next(SHORT), None);
        assert!(preview.gpu().is_none());
    }

    /// The real render thread handles wakes while it has no window to draw into: it stays
    /// responsive, presents nothing and shares its context.
    #[test]
    fn an_offscreen_preview_wakes_but_never_presents() {
        let preview = match PreviewHandle::spawn(PreviewSetup::new(empty_project())) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        let gpu = preview.gpu().expect("shared context");
        assert_eq!(gpu.info().adapter, preview.gpu_info().adapter);
        preview.snapshot_changed();
        preview.seek(Time::from_seconds(1), SeekMode::Exact);
        preview.frame_ready_notifier().notify();
        preview.set_visible(true);
        preview.request_redraw();
        assert!(
            preview
                .render_frame(16, 9, Duration::from_secs(30))
                .is_some()
        );
        assert_eq!(preview.presented_frames(), 0);
    }

    #[test]
    fn device_loss_policy_retries_hardware_then_software_then_stops() {
        assert_eq!(after_device_loss(1), Some(AdapterMode::Auto));
        assert_eq!(after_device_loss(2), Some(AdapterMode::SoftwareOnly));
        assert_eq!(after_device_loss(3), None);
        assert_eq!(after_device_loss(u32::MAX), None);
    }

    /// Runs the recovery path on an offscreen preview: the context is recreated, the callback
    /// reports `DeviceLost`, and frames still render. Repeated losses end with an unavailable
    /// preview that answers frame requests with `None`.
    #[test]
    fn recovers_from_a_lost_device() {
        let (changed_tx, changed) = unbounded();
        let on_changed: GpuChanged = Box::new(move |info, available| {
            let _ = changed_tx.send((info, available));
        });
        let preview = match PreviewHandle::spawn(PreviewSetup {
            on_gpu_changed: on_changed,
            ..PreviewSetup::new(empty_project())
        }) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        assert_ne!(
            preview.gpu_info().fallback,
            Some(FallbackReason::DeviceLost)
        );
        let first = preview.gpu().expect("shared context");

        preview.simulate_device_lost(0);
        let (info, available) = changed
            .recv_timeout(Duration::from_secs(30))
            .expect("gpu changed");
        assert!(available);
        assert_eq!(info.fallback, Some(FallbackReason::DeviceLost));
        assert_eq!(*preview.gpu_info(), info);
        assert!(preview.is_available());
        let second = preview.gpu().expect("recreated context");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "a new context after the loss"
        );
        assert_eq!(second.fallback, Some(FallbackReason::DeviceLost));
        let frame = preview
            .render_frame(64, 36, Duration::from_secs(30))
            .expect("frame after recovery");
        assert_eq!(frame.rgba.len(), 64 * 36 * 4);

        // A stale message from the replaced context is ignored.
        preview.simulate_device_lost(0);
        assert!(
            preview
                .render_frame(8, 8, Duration::from_secs(30))
                .is_some()
        );
        assert!(changed.try_recv().is_err());

        // Second loss: software only (may be missing on this machine, which counts as a third
        // loss); then the preview gives up.
        preview.simulate_device_lost(1);
        let (_, available) = changed
            .recv_timeout(Duration::from_secs(30))
            .expect("gpu changed");
        if available {
            assert!(preview.gpu_info().software);
            preview.simulate_device_lost(2);
            let (_, available) = changed
                .recv_timeout(Duration::from_secs(30))
                .expect("gpu changed");
            assert!(!available);
        }
        assert!(!preview.is_available());
        assert!(preview.gpu().is_none());
        assert!(
            preview
                .render_frame(8, 8, Duration::from_secs(30))
                .is_none()
        );
    }

    /// The offscreen frame is the compositor's output at the playhead, letterboxed: a 16:9
    /// colour clip in a square frame, black bars above and below.
    #[test]
    fn offscreen_frames_show_the_timeline_at_the_playhead() {
        let mut project = project_of(2);
        project.tracks[0].clips[0].source = app_core::ClipSource::Color {
            color: app_core::Rgba8::rgb(200, 40, 10),
        };
        project.background = app_core::CanvasBackground::Color {
            color: app_core::Rgba8::rgb(0, 0, 255),
        };
        let preview = match PreviewHandle::spawn(PreviewSetup::new(snapshot(project))) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        let pixel = |frame: &OffscreenFrame, x: u32, y: u32| {
            let i = ((y * frame.width + x) * 4) as usize;
            [
                frame.rgba[i],
                frame.rgba[i + 1],
                frame.rgba[i + 2],
                frame.rgba[i + 3],
            ]
        };
        let frame = preview
            .render_frame(64, 64, Duration::from_secs(30))
            .expect("frame");
        assert_eq!(pixel(&frame, 32, 32), [200, 40, 10, 255], "the clip");
        assert_eq!(pixel(&frame, 32, 2), [0, 0, 0, 255], "letterbox");
        // After the clip: the background.
        preview.seek(Time::from_seconds(2), SeekMode::Exact);
        let frame = preview
            .render_frame(64, 64, Duration::from_secs(30))
            .expect("frame");
        assert_eq!(pixel(&frame, 32, 32), [0, 0, 255, 255], "the background");
    }

    #[test]
    fn renders_frames_on_request() {
        let preview = match PreviewHandle::spawn(PreviewSetup::new(empty_project())) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        let frame = preview
            .render_frame(320, 180, Duration::from_secs(30))
            .expect("frame");
        assert_eq!((frame.width, frame.height), (320, 180));
        assert_eq!(frame.rgba.len(), 320 * 180 * 4);
    }

    /// Playback with the preview hidden (offscreen): preroll ends, the wall clock runs, ticks
    /// flow at most at 60 Hz, the end pauses on the last frame and that tick arrives.
    #[test]
    fn hidden_playback_ticks_and_pauses_on_the_last_frame() {
        let (tick_tx, ticks) = unbounded();
        let project = snapshot(project_of(1));
        let preview = match PreviewHandle::spawn(PreviewSetup {
            on_tick: Box::new(move |status| {
                let _ = tick_tx.send((Instant::now(), status));
            }),
            ..PreviewSetup::new(project)
        }) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        let start = Instant::now();
        preview.playback().toggle();
        let mut seen = Vec::new();
        let last = loop {
            let (at, status) = ticks.recv_timeout(Duration::from_secs(10)).expect("a tick");
            seen.push((at, status));
            if status.state == PlayState::Paused && seen.len() > 1 {
                break status;
            }
        };
        let frame = Time(app_core::FLICKS_PER_SECOND / 30);
        assert_eq!(last.time, Time::from_seconds(1) - frame, "the last frame");
        assert_eq!(last.rate, 0);
        assert!(seen.iter().any(|(_, s)| s.state == PlayState::Playing));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
        // At most 60 Hz on average (plus the ticks of the state changes).
        let span = seen.last().unwrap().0 - seen[0].0;
        let limit = (span.as_secs_f64() * 60.0).ceil() as usize + 3;
        assert!(seen.len() <= limit, "{} ticks in {span:?}", seen.len());
        assert!(seen.len() >= 20, "{} ticks in {span:?}", seen.len());
        let times: Vec<_> = seen.iter().map(|(_, s)| s.time).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
        // Paused: no more ticks.
        assert!(ticks.recv_timeout(Duration::from_millis(200)).is_err());
    }

    const VSYNC_60: Duration = Duration::from_micros(16_667);

    /// Presents once `interval` after `*now`, returning a finished window's report.
    fn present(
        stats: &mut PresentStats,
        now: &mut Instant,
        interval: Duration,
    ) -> Option<PresentReport> {
        *now += interval;
        stats.record(*now)
    }

    /// Presents at `interval` until a window is reported.
    fn present_until_report(
        stats: &mut PresentStats,
        now: &mut Instant,
        interval: Duration,
    ) -> PresentReport {
        loop {
            if let Some(report) = present(stats, now, interval) {
                return report;
            }
        }
    }

    #[test]
    fn steady_60_hz_has_no_long_gaps() {
        let mut stats = PresentStats::new();
        let mut now = Instant::now();
        assert_eq!(
            stats.record(now),
            None,
            "the first present only starts the window"
        );

        let report = present_until_report(&mut stats, &mut now, VSYNC_60);
        assert_eq!(report.frames, 300);
        assert!(report.window >= PRESENT_STATS_WINDOW);
        assert!((report.avg_fps - 60.0).abs() < 0.01, "{report:?}");
        assert_eq!(report.max_gap, VSYNC_60);
        assert_eq!(report.p99_gap, VSYNC_60);
        assert_eq!(report.long_gaps, 0);
    }

    #[test]
    fn one_hitch_counts_as_a_long_gap() {
        let mut stats = PresentStats::new();
        let mut now = Instant::now();
        stats.record(now);
        for _ in 0..100 {
            assert_eq!(present(&mut stats, &mut now, VSYNC_60), None);
        }
        assert_eq!(
            present(&mut stats, &mut now, Duration::from_millis(50)),
            None
        );

        let report = present_until_report(&mut stats, &mut now, VSYNC_60);
        assert_eq!(report.long_gaps, 1);
        assert_eq!(report.max_gap, Duration::from_millis(50));
        assert_eq!(
            report.p99_gap, VSYNC_60,
            "a single hitch is above the 99th percentile"
        );
        assert!(report.avg_fps < 60.0 && report.avg_fps > 59.0, "{report:?}");
    }

    #[test]
    fn windows_are_continuous() {
        let mut stats = PresentStats::new();
        let mut now = Instant::now();
        stats.record(now);
        let first = present_until_report(&mut stats, &mut now, VSYNC_60);
        // The present that closed the first window starts the second one.
        let second = present_until_report(&mut stats, &mut now, VSYNC_60);
        assert_eq!(first.frames, second.frames);
        assert_eq!(second.long_gaps, 0);
    }

    #[test]
    fn reset_drops_the_window_and_the_pause() {
        let mut stats = PresentStats::new();
        let mut now = Instant::now();
        stats.record(now);
        for _ in 0..200 {
            present(&mut stats, &mut now, VSYNC_60);
        }

        // Hidden for 2 s, then shown again.
        stats.reset();
        now += Duration::from_secs(2);
        assert_eq!(
            stats.record(now),
            None,
            "the first present after a reset starts a window"
        );

        let report = present_until_report(&mut stats, &mut now, VSYNC_60);
        assert_eq!(report.frames, 300, "frames before the reset are dropped");
        assert_eq!(report.max_gap, VSYNC_60, "the pause is not a gap");
        assert_eq!(report.long_gaps, 0);
    }

    #[test]
    fn a_full_buffer_reports_early() {
        let mut stats = PresentStats::new();
        let capacity = stats.intervals.capacity();
        let mut now = Instant::now();
        stats.record(now);
        let report = present_until_report(&mut stats, &mut now, Duration::from_millis(1));
        assert_eq!(report.frames, PRESENT_STATS_CAPACITY);
        assert!(report.window < PRESENT_STATS_WINDOW);
        assert!((report.avg_fps - 1000.0).abs() < 0.01, "{report:?}");
        assert_eq!(stats.intervals.capacity(), capacity, "no reallocation");
    }
}
