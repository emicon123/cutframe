//! Video readers and the frame provider of the render graph (render-graph.md §4.1, §5.3, §5.4,
//! §6, §10; playback.md §7).
//!
//! [`ReaderPool`] implements [`FrameProvider`] over a project snapshot. It opens a reader thread
//! per (media, video stream) on first use, at most [`ReaderOptions::max_readers`] of them, of
//! which at most [`ReaderOptions::max_hw_readers`] decode in hardware (D3D12VA on the render
//! device, zero-copy when the device samples NV12/P010). Readers that the frames of the last
//! 5 s did not use are closed. Images are decoded once on a worker thread.
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
use self::worker::{Reader, ReaderConfig, ReaderSource};
use super::error::{MediaError, MediaErrorKind};
use crate::render::GpuContext;
use crate::render::frames::{FetchPolicy, FrameLookup, FrameProvider, PlaceholderKind};

#[cfg(windows)]
use super::d3d12::{HwDevice, MIN_HW_EXTENT, READER_QUEUE_DEPTH};
#[cfg(not(windows))]
const READER_QUEUE_DEPTH: usize = 6;
#[cfg(not(windows))]
const MIN_HW_EXTENT: i32 = 256;

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
    mode: Mode,
    frame: u64,
}

impl std::fmt::Debug for ReaderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderPool")
            .field("options", &self.options)
            .field("readers", &self.readers.len())
            .field("hw_readers", &self.hw_slots.used())
            .field("mode", &self.mode)
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
            mode: Mode::Paused,
            frame: 0,
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

    /// Playing: readers decode ahead until their queues are full. Paused (the default): the
    /// exact frame plus 2 ahead, then they stop.
    pub fn set_playing(&mut self, playing: bool) {
        self.mode = if playing { Mode::Playing } else { Mode::Paused };
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

    fn close_idle(&mut self, now: Instant) {
        self.readers
            .retain(|reader| now.duration_since(reader.last_used) < IDLE_CLOSE);
    }

    fn spawn(&self, media: MediaId, binding: &Binding) -> Option<Reader> {
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
        let config = ReaderConfig {
            source: ReaderSource {
                media,
                source: binding.source,
                path: binding.path.clone(),
                stream: video.stream_index,
                origin: binding.origin,
            },
            path,
            #[cfg(windows)]
            hw,
            software_threads: self.software_threads,
            depth: READER_QUEUE_DEPTH,
            byte_cap: CPU_QUEUE_BYTES,
            memory: Arc::clone(&self.memory),
            notifier: Arc::clone(&self.notifier),
            below_normal_priority: self.options.below_normal_priority,
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

    /// The reader for `media` at `t` (playback.md §7 reader assignment, without lookahead):
    /// one that gets there without a seek, else an idle reader of the media, else a new one
    /// under the limit, else the least recently used idle reader, reassigned.
    fn pick(&mut self, media: MediaId, binding: &Binding, t: Time) -> Option<usize> {
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
        let mine = |r: &Reader| r.serves(media, source);
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
        let lru = |readers: &[Reader], filter: &dyn Fn(&Reader) -> bool| {
            readers
                .iter()
                .enumerate()
                .filter(|(_, r)| filter(r))
                .min_by_key(|(_, r)| r.last_used)
                .map(|(index, _)| index)
        };
        if let Some(index) = lru(&self.readers, &|r| mine(r) && idle(r)) {
            return Some(index);
        }
        if self.readers.len() >= self.options.max_readers {
            let index = lru(&self.readers, &idle).or_else(|| lru(&self.readers, &mine))?;
            if mine(&self.readers[index]) {
                return Some(index);
            }
            self.readers.remove(index);
        }
        let reader = self.spawn(media, binding)?;
        self.readers.push(reader);
        Some(self.readers.len() - 1)
    }
}

impl FrameProvider for ReaderPool {
    fn begin_frame(&mut self) {
        self.frame += 1;
        self.notifier.rearm();
        self.close_idle(Instant::now());
    }

    fn video(&mut self, media: MediaId, sample_time: Time, policy: FetchPolicy) -> FrameLookup {
        let Some(binding) = self.bindings.get(&media).cloned() else {
            tracing::debug!(%media, "video lookup of a media that is not in the project");
            return FrameLookup::Failed(PlaceholderKind::Offline);
        };
        if binding.video.is_none() {
            return FrameLookup::Failed(PlaceholderKind::DecodeError);
        }
        let Some(index) = self.pick(media, &binding, sample_time) else {
            return FrameLookup::Missing;
        };
        let mode = self.mode;
        let frame = self.frame;
        let reader = &mut self.readers[index];
        reader.last_used = Instant::now();
        reader.used_in_frame = frame;
        reader.lookup(sample_time, mode, policy)
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
