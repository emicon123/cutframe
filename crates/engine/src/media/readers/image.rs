//! Image layers (render-graph.md §5.3): each image is decoded once on a worker thread (CPU,
//! software), downscaled to a long side of at most twice the canvas long side, and kept as one
//! packed RGBA8 frame with straight alpha in stored orientation. HEIC tile grids are stitched by
//! `media::still`. The compositor converts the frame once and caches the texture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use app_core::Time;
use app_core::media::{MediaId, VideoStreamInfo};

use super::{EXACT_TIMEOUT, Notifier, RETRY_AFTER, placeholder_for};
use crate::media::error::MediaError;
use crate::media::frame::{
    CpuFrame, DecodedVideoFrame, FrameData, PlaneBuf, PlaneLayout, SampleDepth,
};
use crate::media::picture::{fit_long_side, frame_to_rgba};
use crate::media::reader::VideoReader;
use crate::media::still::decode_tile_grid;
use crate::render::frames::{FetchPolicy, FrameId, FrameLookup, FrameRef, PlaceholderKind};

/// Decoded images kept in memory, least recently used first out.
const IMAGE_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// The size a `size` picture is decoded at: unchanged up to `max_long_side`, else fitted.
pub(crate) fn capped_size(size: (u32, u32), max_long_side: u32) -> (u32, u32) {
    if size.0.max(size.1) <= max_long_side {
        size
    } else {
        fit_long_side(size.0, size.1, max_long_side)
    }
}

fn packed(width: u32, height: u32, rgba: Vec<u8>) -> CpuFrame {
    CpuFrame {
        layout: PlaneLayout::Packed,
        depth: SampleDepth::U8,
        visible: (width, height),
        planes: [
            Some(PlaneBuf {
                stride: width as usize * 4,
                bytes: rgba,
            }),
            None,
            None,
        ],
    }
}

/// Decodes the picture of an image (or the first frame of any video stream) as packed RGBA8,
/// straight alpha, stored orientation, long side at most `max_long_side`.
pub(crate) fn decode_image(
    path: &Path,
    video: &VideoStreamInfo,
    origin: Time,
    max_long_side: u32,
    cancel: &AtomicBool,
) -> Result<CpuFrame, MediaError> {
    if let Some(grid) = decode_tile_grid(path, cancel)? {
        let (w, h) = capped_size((grid.width, grid.height), max_long_side);
        return Ok(packed(w, h, grid.resized(w, h)?.to_rgba()));
    }
    let mut reader = VideoReader::open(path, video.stream_index, origin, cancel)?;
    let (frame, _) = reader.frame_at(Time::ZERO, None)?;
    let (w, h) = capped_size(
        (frame.width.max(1) as u32, frame.height.max(1) as u32),
        max_long_side,
    );
    Ok(packed(w, h, frame_to_rgba(&frame, w, h)?))
}

/// One image to decode.
#[derive(Debug, Clone)]
pub(crate) struct ImageJob {
    pub media: MediaId,
    /// Identity of the file, stream and decode size ([`FrameId::source`]).
    pub source: u64,
    pub path: PathBuf,
    pub video: VideoStreamInfo,
    pub origin: Time,
    pub max_long_side: u32,
}

#[derive(Debug)]
enum State {
    Pending,
    Ready(FrameRef),
    Failed(PlaceholderKind, Instant),
}

#[derive(Debug)]
struct Entry {
    source: u64,
    state: State,
    /// The frame of an earlier source, shown as stale while the new one decodes.
    previous: Option<FrameRef>,
    last_used: Instant,
}

#[derive(Debug)]
struct Shared {
    entries: Mutex<HashMap<MediaId, Entry>>,
    cond: Condvar,
    cancel: AtomicBool,
    notifier: Arc<Notifier>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, HashMap<MediaId, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Decoded images and the worker thread that decodes them.
#[derive(Debug)]
pub(crate) struct ImageCache {
    shared: Arc<Shared>,
    jobs: Option<crossbeam_channel::Sender<ImageJob>>,
    worker: Option<JoinHandle<()>>,
}

impl ImageCache {
    pub fn new(notifier: Arc<Notifier>) -> Self {
        Self {
            shared: Arc::new(Shared {
                entries: Mutex::new(HashMap::new()),
                cond: Condvar::new(),
                cancel: AtomicBool::new(false),
                notifier,
            }),
            jobs: None,
            worker: None,
        }
    }

    /// The decoded picture of `job.media`; queues the decode on first use.
    pub fn lookup(&mut self, job: ImageJob, policy: FetchPolicy) -> FrameLookup {
        let shared = Arc::clone(&self.shared);
        let mut entries = shared.lock();
        let now = Instant::now();
        let submit = match entries.get_mut(&job.media) {
            Some(entry) if entry.source == job.source => match entry.state {
                State::Failed(_, at) if now.duration_since(at) >= RETRY_AFTER => {
                    entry.state = State::Pending;
                    true
                }
                _ => false,
            },
            Some(entry) => {
                if let State::Ready(frame) = &entry.state {
                    entry.previous = Some(frame.clone());
                }
                entry.source = job.source;
                entry.state = State::Pending;
                true
            }
            None => {
                entries.insert(
                    job.media,
                    Entry {
                        source: job.source,
                        state: State::Pending,
                        previous: None,
                        last_used: now,
                    },
                );
                true
            }
        };
        let media = job.media;
        if submit {
            self.submit(job);
        }
        let deadline = now + EXACT_TIMEOUT;
        loop {
            let Some(entry) = entries.get_mut(&media) else {
                return FrameLookup::Missing;
            };
            entry.last_used = now;
            match &entry.state {
                State::Ready(frame) => return FrameLookup::Exact(frame.clone()),
                State::Failed(kind, _) => return FrameLookup::Failed(*kind),
                State::Pending if policy == FetchPolicy::BestAvailable => {
                    shared.notifier.rearm();
                    return entry
                        .previous
                        .clone()
                        .map_or(FrameLookup::Missing, FrameLookup::Stale);
                }
                State::Pending => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        tracing::warn!(%media, "image decode timed out");
                        return FrameLookup::Failed(PlaceholderKind::DecodeError);
                    }
                    entries = shared
                        .cond
                        .wait_timeout(entries, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        }
    }

    fn submit(&mut self, job: ImageJob) {
        if self.jobs.is_none() {
            let (sender, receiver) = crossbeam_channel::unbounded::<ImageJob>();
            let shared = Arc::clone(&self.shared);
            match std::thread::Builder::new()
                .name("image-decode".into())
                .spawn(move || {
                    for job in receiver {
                        decode_job(&shared, job);
                    }
                }) {
                Ok(handle) => {
                    self.jobs = Some(sender);
                    self.worker = Some(handle);
                }
                Err(err) => tracing::error!(%err, "cannot start the image decode thread"),
            }
        }
        if let Some(jobs) = &self.jobs
            && jobs.send(job.clone()).is_ok()
        {
            return;
        }
        finish(
            &self.shared,
            &job,
            State::Failed(PlaceholderKind::DecodeError, Instant::now()),
        );
    }

    /// Forgets every decoded image.
    pub fn clear(&mut self) {
        self.shared.lock().clear();
    }
}

impl Drop for ImageCache {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            worker.join().ok();
        }
    }
}

fn decode_job(shared: &Shared, job: ImageJob) {
    // Skip jobs whose entry moved on (relinked, resized, cleared) while they waited.
    let current = shared
        .lock()
        .get(&job.media)
        .is_some_and(|e| e.source == job.source && matches!(e.state, State::Pending));
    if !current {
        return;
    }
    let started = Instant::now();
    let result = decode_image(
        &job.path,
        &job.video,
        job.origin,
        job.max_long_side,
        &shared.cancel,
    );
    let state = match result {
        Ok(frame) => {
            tracing::debug!(
                media = %job.media, size = ?frame.visible,
                ms = started.elapsed().as_millis() as u64, "image decoded"
            );
            let id = FrameId {
                media: job.media,
                source: job.source,
                pts: None,
            };
            let frame = DecodedVideoFrame {
                pts: Time::ZERO,
                data: FrameData::Cpu(frame),
            };
            State::Ready(FrameRef::new(id, Arc::new(frame)))
        }
        Err(err) => {
            let kind = placeholder_for(&err);
            match kind {
                PlaceholderKind::Offline => shared.notifier.media_unavailable(job.media, &err),
                PlaceholderKind::DecodeError => shared.notifier.decode_failed(job.media, &err),
            }
            State::Failed(kind, Instant::now())
        }
    };
    finish(shared, &job, state);
}

/// Stores a finished decode, evicts old images over the budget and wakes the consumer.
fn finish(shared: &Shared, job: &ImageJob, state: State) {
    {
        let mut entries = shared.lock();
        let Some(entry) = entries.get_mut(&job.media) else {
            return;
        };
        if entry.source != job.source {
            return;
        }
        entry.state = state;
        entry.previous = None;
        evict(&mut entries, job.media);
    }
    shared.cond.notify_all();
    shared.notifier.frame_ready();
}

fn bytes_of(entry: &Entry) -> usize {
    match &entry.state {
        State::Ready(frame) => frame.data().cpu_bytes(),
        _ => 0,
    }
}

/// Drops the least recently used decoded images (never `keep`) until the rest fit the budget.
fn evict(entries: &mut HashMap<MediaId, Entry>, keep: MediaId) {
    let mut total: usize = entries.values().map(bytes_of).sum();
    while total > IMAGE_CACHE_BYTES {
        let Some((&oldest, entry)) = entries
            .iter()
            .filter(|(id, e)| **id != keep && matches!(e.state, State::Ready(_)))
            .min_by_key(|(_, e)| e.last_used)
        else {
            return;
        };
        total -= bytes_of(entry);
        entries.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_capped_at_the_long_side() {
        assert_eq!(capped_size((64, 48), 3840), (64, 48));
        assert_eq!(capped_size((8064, 6048), 3840), (3840, 2880));
        assert_eq!(capped_size((6048, 8064), 3840), (2880, 3840));
        assert_eq!(capped_size((3840, 100), 3840), (3840, 100));
    }
}
