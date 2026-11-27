//! Video readers and the frame provider of the render graph (render-graph.md §4.1, §5.3, §5.4,
//! §6, §10; playback.md §7).
//!
//! [`ReaderPool`] implements [`FrameProvider`] over a project snapshot. It opens a reader thread
//! per (media, video stream) on first use, at most [`ReaderOptions::max_readers`] of them, of
//! which at most [`ReaderOptions::max_hw_readers`] decode in hardware (D3D12VA on the render
//! device, zero-copy when the device samples NV12/P010). Readers that the frames of the last
//! 5 s did not use are closed. Images are decoded once on a worker thread.
//!
//! Each reader runs forward (decode ahead from a seek point) or, for reversed clips, in
//! GOP-reverse mode (`clip-speed-audio.md` §5.2, `window.rs`): the pool binds readers per media
//! and direction, at most [`MAX_REVERSE_READERS`] of them in reverse mode. How a lookup steers
//! its reader follows the layer's [`Pace`] and the transport rate set with
//! [`ReaderPool::set_rate`].
//!
//! Readers never take the document lock: the owner hands the pool project snapshots
//! ([`ReaderPool::set_project`]), from which it only reads media paths and streams.
//!
//! The owner gets [`ReaderEvent`]s through the callback given to [`ReaderPool::new`], from
//! reader threads: `FrameReady` (coalesced: at most one until the next lookup that waits) and
//! `MediaUnavailable` (once per media and pool).

mod decode_path;
mod image;
mod queue;
mod window;
mod worker;

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_core::media::{MediaId, VideoStreamInfo};
use app_core::{Project, Time};

pub use decode_path::DecodePath;

use self::decode_path::{
    HwSlots, PathInputs, PathMemory, hw_decode_allowed_by_env, initial_path, software_threads_here,
};
use self::image::{ImageCache, ImageJob};
use self::queue::{CPU_QUEUE_BYTES, Mode};
use self::window::REVERSE_WINDOW_BYTES;
use self::worker::{Reader, ReaderConfig, ReaderSource, Steering};
use super::error::{MediaError, MediaErrorKind};
use crate::render::GpuContext;
use crate::render::frames::{FetchPolicy, FrameLookup, FrameProvider, Pace, PlaceholderKind};

#[cfg(windows)]
use super::d3d12::{HwDevice, MIN_HW_EXTENT, READER_QUEUE_DEPTH};
#[cfg(not(windows))]
const READER_QUEUE_DEPTH: usize = 6;
#[cfg(not(windows))]
const MIN_HW_EXTENT: i32 = 256;

/// Readers in GOP-reverse mode at most (counted against [`ReaderOptions::max_readers`]): each
/// holds up to two windows of CPU frames.
pub const MAX_REVERSE_READERS: usize = 2;

/// Above this effective rate (transport rate x clip speed, in percent of normal) a playing
/// reader decodes keyframes only and sample and hold shows the latest (`clip-speed-audio.md`
/// §5.1). Up to it every frame is decoded, with catch-up when behind.
pub const KEYFRAMES_ONLY_ABOVE: u32 = 400;

/// Above this transport rate (shuttle) playing readers decode keyframes only (playback.md §7).
const SHUTTLE_KEYFRAMES_ABOVE: u8 = 2;

/// Readers that no frame used for this long are closed (playback.md §7).
const IDLE_CLOSE: Duration = Duration::from_secs(5);

/// A failed media (offline, decoder failed) is tried again after this long, so a file that
/// comes back (a USB drive plugged in again) shows up without a relink.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The longest an `Exact` lookup waits for one frame before the source counts as failed.
pub(crate) const EXACT_TIMEOUT: Duration = Duration::from_secs(60);

/// What the pool reports to its owner (called from reader threads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderEvent {
    /// A frame that a `BestAvailable` lookup did not have yet arrived (or its reader failed):
    /// draw again. Coalesced: sent at most once until a later lookup has to wait again.
    FrameReady,
    /// The media's file is missing or unreadable. Sent once per media; the shell sets
    /// `MediaStatus::Offline` (media-import.md §6.6).
    MediaUnavailable(MediaId),
}

/// Reader limits (playback.md §7; export uses fewer, render-graph.md §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderOptions {
    pub max_readers: usize,
    pub max_hw_readers: usize,
    /// Reader and image threads run below normal priority, so an export does not starve the
    /// preview (Windows; FFmpeg's own decode threads keep their priority).
    pub below_normal_priority: bool,
}

impl Default for ReaderOptions {
    fn default() -> Self {
        Self {
            max_readers: 8,
            max_hw_readers: 4,
            below_normal_priority: false,
        }
    }
}

/// Lowers the calling thread to below-normal priority (Windows); logs a failure.
pub(crate) fn lower_thread_priority() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: `GetCurrentThread` returns a pseudo handle that is always valid for the
        // calling thread and needs no closing.
        let result = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
        if let Err(err) = result {
            tracing::debug!(%err, "cannot lower the thread priority");
        }
    }
}

/// The placeholder for a source that failed with `err`.
pub(crate) fn placeholder_for(err: &MediaError) -> PlaceholderKind {
    match err.kind {
        MediaErrorKind::FileNotFound
        | MediaErrorKind::AccessDenied
        | MediaErrorKind::UnsupportedPath => PlaceholderKind::Offline,
        _ => PlaceholderKind::DecodeError,
    }
}

/// The owner's event callback, with the coalescing flag and the once-per-media reports.
pub(crate) struct Notifier {
    sink: Box<dyn Fn(ReaderEvent) + Send + Sync>,
    /// `FrameReady` was sent and no lookup has waited since.
    sent: AtomicBool,
    unavailable: Mutex<HashSet<MediaId>>,
    warned: Mutex<HashSet<MediaId>>,
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifier")
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl Notifier {
    fn new(sink: Box<dyn Fn(ReaderEvent) + Send + Sync>) -> Self {
        Self {
            sink,
            sent: AtomicBool::new(false),
            unavailable: Mutex::new(HashSet::new()),
            warned: Mutex::new(HashSet::new()),
        }
    }

    /// Sends `FrameReady` unless one is pending.
    pub fn frame_ready(&self) {
        if !self.sent.swap(true, Ordering::AcqRel) {
            (self.sink)(ReaderEvent::FrameReady);
        }
    }

    /// A lookup is about to wait: the next ready frame sends `FrameReady` again.
    pub fn rearm(&self) {
        self.sent.store(false, Ordering::Release);
    }

    fn first(set: &Mutex<HashSet<MediaId>>, media: MediaId) -> bool {
        set.lock().map(|mut set| set.insert(media)).unwrap_or(false)
    }

    /// Reports an offline media once.
    pub fn media_unavailable(&self, media: MediaId, err: &MediaError) {
        if Self::first(&self.unavailable, media) {
            tracing::info!(%media, %err, "media unavailable");
            (self.sink)(ReaderEvent::MediaUnavailable(media));
        }
    }

    /// Logs a failed decoder once per media.
    pub fn decode_failed(&self, media: MediaId, err: &MediaError) {
        if Self::first(&self.warned, media) {
            tracing::warn!(%media, %err, "decoding failed; showing the placeholder");
        }
    }
}

/// One source frame when the stream's frame rate is unknown.
const DEFAULT_SOURCE_FRAME: Time = Time(app_core::FLICKS_PER_SECOND / 30);

/// What the pool knows about one media of the snapshot.
#[derive(Debug, Clone)]
struct Binding {
    path: PathBuf,
    origin: Time,
    video: Option<VideoStreamInfo>,
    /// [`crate::render::frames::FrameId::source`] of its video frames.
    source: u64,
}

impl Binding {
    fn of(asset: &app_core::MediaAsset) -> Self {
        let video = asset.info.video.clone();
        let mut hasher = DefaultHasher::new();
        asset.path.absolute.hash(&mut hasher);
        video.as_ref().map(|v| v.stream_index).hash(&mut hasher);
        asset.info.origin.hash(&mut hasher);
        Self {
            path: PathBuf::from(&asset.path.absolute),
            origin: asset.info.origin,
            video,
            source: hasher.finish(),
        }
    }

    /// One source frame, from the stream's average frame rate.
    fn source_frame(&self) -> Time {
        self.video
            .as_ref()
            .and_then(|video| video.frame_rate)
            .filter(|rate| rate.num() > 0)
            .map_or(DEFAULT_SOURCE_FRAME, |rate| {
                let flicks =
                    app_core::FLICKS_PER_SECOND * i64::from(rate.den()) / i64::from(rate.num());
                Time(flicks.max(1))
            })
    }

    /// The source id of the image decoded with long side at most `max_long_side`.
    fn image_source(&self, max_long_side: u32) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.source.hash(&mut hasher);
        max_long_side.hash(&mut hasher);
        hasher.finish()
    }
}

/// The readers and decoded images of one consumer (the preview, or one export).
///
/// Call [`FrameProvider::begin_frame`] once per composited frame (the compositor does), then
/// look frames up. `BestAvailable` never blocks; `Exact` blocks until the frame is decoded or
/// the source fails.
pub struct ReaderPool {
    options: ReaderOptions,
    #[cfg(windows)]
    gpu: Option<Arc<GpuContext>>,
    #[cfg(windows)]
    hw_device: Option<Arc<HwDevice>>,
    hw_allowed: bool,
    zero_copy: bool,
    hw_slots: Arc<HwSlots>,
    software_threads: i32,
    memory: Arc<PathMemory>,
    notifier: Arc<Notifier>,
    bindings: HashMap<MediaId, Binding>,
    canvas_long_side: u32,
    readers: Vec<Reader>,
    images: ImageCache,
    /// The transport rate: 0 paused, else the playing rate (playback.md §7).
    rate: i8,
    frame: u64,
    /// Slows every reader down (catch-up tests).
    #[cfg(test)]
    decode_delay: Duration,
    /// Bytes of one reverse window (tests use a tiny cap).
    #[cfg(test)]
    window_bytes: usize,
}

impl std::fmt::Debug for ReaderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderPool")
            .field("options", &self.options)
            .field("readers", &self.readers.len())
            .field("hw_readers", &self.hw_slots.used())
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

impl ReaderPool {
    /// A pool decoding for `gpu` (hardware decode needs it; `None` decodes everything in
    /// software into CPU frames). `events` is called from reader threads.
    pub fn new(
        gpu: Option<Arc<GpuContext>>,
        options: ReaderOptions,
        events: impl Fn(ReaderEvent) + Send + Sync + 'static,
    ) -> Self {
        let hw_allowed = hw_decode_allowed_by_env();
        if !hw_allowed {
            tracing::info!(
                "{} turns hardware video decode off",
                decode_path::HWDECODE_ENV
            );
        }
        #[cfg(windows)]
        let hw_device = match &gpu {
            Some(gpu) if hw_allowed && options.max_hw_readers > 0 => match HwDevice::new(gpu) {
                Ok(device) => Some(Arc::new(device)),
                Err(err) => {
                    tracing::info!(%err, "no hardware video decode; decoding in software");
                    None
                }
            },
            _ => None,
        };
        let zero_copy = gpu.as_ref().is_some_and(|g| g.supports_zero_copy_video());
        let notifier = Arc::new(Notifier::new(Box::new(events)));
        Self {
            options,
            #[cfg(windows)]
            gpu,
            #[cfg(windows)]
            hw_device,
            hw_allowed,
            zero_copy,
            hw_slots: HwSlots::new(options.max_hw_readers),
            software_threads: software_threads_here(),
            memory: Arc::new(PathMemory::default()),
            images: ImageCache::new(Arc::clone(&notifier), options.below_normal_priority),
            notifier,
            bindings: HashMap::new(),
            canvas_long_side: 0,
            readers: Vec::new(),
            rate: 0,
            frame: 0,
            #[cfg(test)]
            decode_delay: Duration::ZERO,
            #[cfg(test)]
            window_bytes: REVERSE_WINDOW_BYTES,
        }
    }

    /// Takes a new project snapshot (only its media and canvas size are read). Readers of media
    /// that are gone or whose file or stream changed (relink) close; images are decoded again
    /// when their size cap changes.
    pub fn set_project(&mut self, project: &Project) {
        self.canvas_long_side = project.settings.width.max(project.settings.height);
        self.bindings = project
            .media
            .iter()
            .map(|asset| (asset.id, Binding::of(asset)))
            .collect();
        let bindings = &self.bindings;
        self.readers.retain(|reader| {
            bindings
                .get(&reader.media())
                .is_some_and(|b| b.source == reader.source())
        });
    }

    /// The transport: 0 paused (the default), else the playing rate (+-1, +-2, +-4, +-8).
    /// Export renders as if playing at 1x. Steers every later lookup (see `steering`).
    pub fn set_rate(&mut self, rate: i8) {
        self.rate = rate;
    }

    /// Which reader direction serves a layer of `pace` (true: GOP-reverse), and how its lookups
    /// steer it (playback.md §7, `clip-speed-audio.md` §5.1):
    /// - A source moving forward while playing decodes ahead (`Mode::Playing`): every frame up to
    ///   an effective rate (transport rate x clip speed) of [`KEYFRAMES_ONLY_ABOVE`] (with
    ///   catch-up), keyframes only above it or above the shuttle's 2x. Back to every frame costs
    ///   a seek to the next keyframe.
    /// - A reversed clip with the transport paused or playing forward goes to a GOP-reverse
    ///   reader, which prepares the next earlier window while playing.
    /// - Reverse transport (J) over a forward clip stays the timed scrub: a forward reader, one
    ///   request per frame (`Mode::Paused`); over a reversed clip the source moves forward.
    /// - `Still` (a freeze frame) and paused: the exact frame and a little ahead.
    /// - `FetchPolicy::Exact` never skips frames.
    fn steering(&self, policy: FetchPolicy, pace: Pace) -> (bool, Steering) {
        let (speed, clip_reversed) = match pace {
            Pace::Forward { speed } => (speed, false),
            Pace::Reverse { speed } => (speed, true),
            Pace::Still => {
                let still = Steering {
                    mode: Mode::Paused,
                    keyframes_only: false,
                    catch_up: false,
                };
                return (false, still);
            }
        };
        let reverse = clip_reversed && self.rate >= 0;
        let playing = self.rate > 0 || (self.rate < 0 && clip_reversed);
        let exact = policy == FetchPolicy::Exact;
        let effective = u32::from(self.rate.unsigned_abs()) * u32::from(speed.percent());
        let keyframes_only = playing
            && !exact
            && (self.rate.unsigned_abs() > SHUTTLE_KEYFRAMES_ABOVE
                || effective > KEYFRAMES_ONLY_ABOVE);
        let steering = Steering {
            mode: if playing { Mode::Playing } else { Mode::Paused },
            keyframes_only,
            catch_up: playing && !exact && !keyframes_only && !reverse,
        };
        (reverse, steering)
    }

    /// The playback scheduler (playback.md §7): a clip of `media` starts soon and needs
    /// `sample_time` first. Call after the lookups of the current frame. A reader already
    /// primed for it is kept; else an idle reader of the media decodes towards it now (without
    /// a seek when it gets there by decoding forward), else a new reader opens within the
    /// limits, else the least recently used idle reader is reassigned. Readers that serve the
    /// current frame are never taken.
    pub fn prime(&mut self, media: MediaId, sample_time: Time, pace: Pace) {
        let Some(binding) = self.bindings.get(&media).cloned() else {
            return;
        };
        if binding.video.is_none() {
            return;
        }
        let source = binding.source;
        let frame = self.frame;
        let (reverse, steering) = self.steering(FetchPolicy::BestAvailable, pace);
        let mine =
            |r: &Reader| r.serves(media, source) && r.reverse == reverse && r.failed().is_none();
        // Free: not serving this frame and not primed for another clip in this pass.
        let free = |r: &Reader| r.used_in_frame != frame && r.primed_in_frame != frame;
        if let Some(reader) = self
            .readers
            .iter_mut()
            .find(|r| mine(r) && r.primed == Some(sample_time))
        {
            reader.primed_in_frame = frame;
            reader.last_used = Instant::now();
            return;
        }
        let reachable = self
            .readers
            .iter()
            .enumerate()
            .filter(|(_, r)| mine(r) && free(r))
            .filter_map(|(index, r)| r.distance(sample_time).map(|d| (d, index)))
            .min()
            .map(|(_, index)| index);
        let index = reachable
            .or_else(|| lru(&self.readers, &|r| mine(r) && free(r)))
            .or_else(|| {
                if reverse && self.reverse_readers() >= MAX_REVERSE_READERS {
                    let index = lru(&self.readers, &|r| r.reverse && free(r))?;
                    self.readers.remove(index);
                }
                if self.readers.len() >= self.options.max_readers {
                    let index = lru(&self.readers, &free)?;
                    self.readers.remove(index);
                }
                let reader = self.spawn(media, &binding, reverse)?;
                self.readers.push(reader);
                Some(self.readers.len() - 1)
            });
        if let Some(index) = index {
            tracing::trace!(%media, ?sample_time, "priming a video reader");
            self.readers[index].prime(sample_time, steering, frame);
        }
    }

    /// Seeks the readers of `media` started so far (diagnostics, tests).
    pub fn seek_count(&self, media: MediaId) -> u32 {
        self.readers
            .iter()
            .filter(|r| r.media() == media)
            .map(Reader::seeks)
            .sum()
    }

    /// Catch-up jumps of the readers of `media` so far (diagnostics, tests).
    pub fn catch_up_count(&self, media: MediaId) -> u32 {
        self.readers
            .iter()
            .filter(|r| r.media() == media)
            .map(Reader::catch_ups)
            .sum()
    }

    /// Closes every reader and forgets every decoded image (e.g. when the preview is hidden).
    pub fn close_all(&mut self) {
        self.readers.clear();
        self.images.clear();
    }

    /// The decode path of the most recently used reader of `media` (diagnostics, tests).
    pub fn decode_path(&self, media: MediaId) -> Option<DecodePath> {
        self.readers
            .iter()
            .filter(|r| r.media() == media)
            .max_by_key(|r| r.last_used)
            .map(Reader::decode_path)
    }

    /// Open readers.
    pub fn reader_count(&self) -> usize {
        self.readers.len()
    }

    /// Open readers in GOP-reverse mode.
    pub fn reverse_readers(&self) -> usize {
        self.readers.iter().filter(|r| r.reverse).count()
    }

    fn close_idle(&mut self, now: Instant) {
        self.readers
            .retain(|reader| now.duration_since(reader.last_used) < IDLE_CLOSE);
    }

    fn spawn(&self, media: MediaId, binding: &Binding, reverse: bool) -> Option<Reader> {
        let video = binding.video.as_ref()?;
        let path = initial_path(PathInputs {
            hw_allowed: self.hw_allowed,
            hw_device: self.has_hw_device(),
            zero_copy: self.zero_copy,
            extent: (video.width, video.height),
            min_extent: MIN_HW_EXTENT as u32,
            remembered: self.memory.get(media),
        });
        #[cfg(windows)]
        let hw = match (path, &self.hw_device, &self.gpu) {
            (DecodePath::Software, ..) => None,
            (_, Some(device), Some(gpu)) => {
                self.hw_slots.try_acquire().map(|slot| worker::HwContext {
                    device: Arc::clone(device),
                    gpu: Arc::clone(gpu),
                    slot,
                })
            }
            _ => None,
        };
        #[cfg(windows)]
        let path = if hw.is_some() {
            path
        } else {
            DecodePath::Software
        };
        #[cfg(not(windows))]
        let path = DecodePath::Software;
        // A reverse window outlives the decoder's small surface pool: hardware frames are
        // downloaded into the window's CPU ring.
        let path = if reverse && path == DecodePath::ZeroCopy {
            DecodePath::Download
        } else {
            path
        };
        #[cfg(test)]
        let window_bytes = self.window_bytes;
        #[cfg(not(test))]
        let window_bytes = REVERSE_WINDOW_BYTES;
        let config = ReaderConfig {
            source: ReaderSource {
                media,
                source: binding.source,
                path: binding.path.clone(),
                stream: video.stream_index,
                origin: binding.origin,
                frame: binding.source_frame(),
            },
            path,
            #[cfg(windows)]
            hw,
            software_threads: self.software_threads,
            depth: READER_QUEUE_DEPTH,
            byte_cap: if reverse {
                window_bytes
            } else {
                CPU_QUEUE_BYTES
            },
            reverse,
            memory: Arc::clone(&self.memory),
            notifier: Arc::clone(&self.notifier),
            below_normal_priority: self.options.below_normal_priority,
            #[cfg(test)]
            decode_delay: self.decode_delay,
        };
        Reader::spawn(config)
            .inspect_err(|err| tracing::error!(%err, "cannot start a video reader thread"))
            .ok()
    }

    fn has_hw_device(&self) -> bool {
        #[cfg(windows)]
        return self.hw_device.is_some();
        #[cfg(not(windows))]
        false
    }

    /// The reader for `media` at `t` in direction `reverse` (playback.md §7 reader assignment,
    /// without lookahead): one that gets there without a seek, else an idle reader of the
    /// media, else a new one under the limits, else the least recently used idle reader,
    /// reassigned.
    fn pick(&mut self, media: MediaId, binding: &Binding, t: Time, reverse: bool) -> Option<usize> {
        let source = binding.source;
        let frame = self.frame;
        let now = Instant::now();
        // A failed source answers until it is retried.
        if let Some(index) = self
            .readers
            .iter()
            .position(|r| r.serves(media, source) && r.failed().is_some())
        {
            match self.readers[index].failed() {
                Some((_, at)) if now.duration_since(at) < RETRY_AFTER => return Some(index),
                _ => {
                    self.readers.remove(index);
                }
            }
        }
        let mine = |r: &Reader| r.serves(media, source) && r.reverse == reverse;
        let idle = |r: &Reader| r.used_in_frame != frame;
        // Another layer of this frame already asked for exactly this time.
        if let Some(index) = self
            .readers
            .iter()
            .position(|r| mine(r) && r.target() == Some(t))
        {
            return Some(index);
        }
        // Reached by decoding forward; readers busy with another time in this frame are kept.
        let nearest = self
            .readers
            .iter()
            .enumerate()
            .filter(|(_, r)| mine(r) && idle(r))
            .filter_map(|(index, r)| r.distance(t).map(|d| (d, index)))
            .min();
        if let Some((_, index)) = nearest {
            return Some(index);
        }
        if let Some(index) = lru(&self.readers, &|r| mine(r) && idle(r)) {
            return Some(index);
        }
        if reverse && self.reverse_readers() >= MAX_REVERSE_READERS {
            let index = lru(&self.readers, &|r| r.reverse && idle(r))
                .or_else(|| lru(&self.readers, &mine))?;
            if mine(&self.readers[index]) {
                return Some(index);
            }
            self.readers.remove(index);
        }
        if self.readers.len() >= self.options.max_readers {
            let index = lru(&self.readers, &idle).or_else(|| lru(&self.readers, &mine))?;
            if mine(&self.readers[index]) {
                return Some(index);
            }
            self.readers.remove(index);
        }
        let reader = self.spawn(media, binding, reverse)?;
        self.readers.push(reader);
        Some(self.readers.len() - 1)
    }
}

/// The least recently used reader that passes `filter`, preferring readers the scheduler has
/// not primed for an upcoming clip.
fn lru(readers: &[Reader], filter: &dyn Fn(&Reader) -> bool) -> Option<usize> {
    let pick = |primed: bool| {
        readers
            .iter()
            .enumerate()
            .filter(|(_, r)| filter(r) && r.primed.is_some() == primed)
            .min_by_key(|(_, r)| r.last_used)
            .map(|(index, _)| index)
    };
    pick(false).or_else(|| pick(true))
}

impl FrameProvider for ReaderPool {
    fn begin_frame(&mut self) {
        self.frame += 1;
        self.notifier.rearm();
        self.close_idle(Instant::now());
    }

    fn video(
        &mut self,
        media: MediaId,
        sample_time: Time,
        pace: Pace,
        policy: FetchPolicy,
    ) -> FrameLookup {
        let Some(binding) = self.bindings.get(&media).cloned() else {
            tracing::debug!(%media, "video lookup of a media that is not in the project");
            return FrameLookup::Failed(PlaceholderKind::Offline);
        };
        if binding.video.is_none() {
            return FrameLookup::Failed(PlaceholderKind::DecodeError);
        }
        let (reverse, steering) = self.steering(policy, pace);
        let Some(index) = self.pick(media, &binding, sample_time, reverse) else {
            return FrameLookup::Missing;
        };
        let frame = self.frame;
        let reader = &mut self.readers[index];
        reader.last_used = Instant::now();
        reader.used_in_frame = frame;
        reader.lookup(sample_time, steering, policy)
    }

    fn image(&mut self, media: MediaId, policy: FetchPolicy) -> FrameLookup {
        let Some(binding) = self.bindings.get(&media) else {
            tracing::debug!(%media, "image lookup of a media that is not in the project");
            return FrameLookup::Failed(PlaceholderKind::Offline);
        };
        let Some(video) = binding.video.clone() else {
            return FrameLookup::Failed(PlaceholderKind::DecodeError);
        };
        let max_long_side = match self.canvas_long_side {
            0 => u32::MAX,
            long => long.saturating_mul(2),
        };
        let job = ImageJob {
            media,
            source: binding.image_source(max_long_side),
            path: binding.path.clone(),
            video,
            origin: binding.origin,
            max_long_side,
        };
        self.images.lookup(job, policy)
    }
}

#[cfg(test)]
mod tests;
