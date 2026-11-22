//! Media panel previews (posters, filmstrip tiles, waveforms): the disk cache, the in-flight map
//! that makes concurrent requests for one product share one generation, and the background
//! prefetch job (design §6.3, §6.4, review F12).
//!
//! - A product is keyed by (fingerprint hash, product), so it survives moves and restarts and a
//!   changed file never shows stale pictures.
//! - [`Previews::attach`] adds a waiter for a product; the first waiter's caller generates it with
//!   [`Previews::produce`] on a pool worker and every waiter gets the result.
//! - Failures other than "file not reachable" are remembered for the session (`500`), so the UI
//!   does not retry in a loop. Unreachable files (`503`) are retried on the next request.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use app_core::media::FilmstripLayout;
use app_core::{MediaAsset, MediaId, Project};
use engine::media::cache::{PreviewCache, Product};
use engine::media::{MediaError, MediaErrorKind, previews};

use crate::ipc::IpcError;
use crate::ipc::types::{JobId, JobStage};
use crate::jobs::{JobContext, JobManager, JobSpawner, Priority};

/// Produces the bytes of one product: the engine's generators in the app, a fake in tests.
pub type Generator =
    dyn Fn(&Path, &MediaAsset, Product, &AtomicBool) -> Result<Vec<u8>, MediaError> + Send + Sync;

/// One product of one content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub hash: String,
    pub product: Product,
}

impl Key {
    pub fn new(asset: &MediaAsset, product: Product) -> Self {
        Self {
            hash: asset.fingerprint.hash.clone(),
            product,
        }
    }
}

/// What a waiter gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(Arc<[u8]>),
    /// Generation failed for a reason that will not go away this session (`500`).
    Failed,
    /// The file could not be reached (offline, locked) or the work was cancelled (`503`).
    Unavailable,
}

/// Called once with the outcome of a product.
pub type Waiter = Box<dyn FnOnce(&Outcome) + Send>;

/// `?v=` of every `media://` URL of `asset`: the first 16 hex chars of its content hash.
pub fn preview_version(asset: &MediaAsset) -> String {
    asset.fingerprint.hash.chars().take(16).collect()
}

/// The products of `asset` in prefetch order groups: poster, waveform, filmstrip tiles.
pub fn products(asset: &MediaAsset) -> (Option<Product>, Option<Product>, Vec<Product>) {
    let poster = asset.has_poster().then_some(Product::Poster);
    let waveform = asset.has_waveform().then_some(Product::Waveform);
    let tiles = FilmstripLayout::of(asset)
        .map(|layout| (0..layout.tiles()).map(Product::FilmstripTile).collect())
        .unwrap_or_default();
    (poster, waveform, tiles)
}

/// Whether `asset` has `product` (e.g. tile 7 of a 3-tile filmstrip does not).
pub fn has_product(asset: &MediaAsset, product: Product) -> bool {
    match product {
        Product::Poster => asset.has_poster(),
        Product::Waveform => asset.has_waveform(),
        Product::FilmstripTile(tile) => {
            FilmstripLayout::of(asset).is_some_and(|layout| tile < layout.tiles())
        }
    }
}

fn engine_generator(
    path: &Path,
    asset: &MediaAsset,
    product: Product,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, MediaError> {
    match product {
        Product::Poster => previews::poster(path, asset, cancel),
        Product::FilmstripTile(tile) => previews::filmstrip_tile(path, asset, tile, cancel),
        Product::Waveform => previews::asset_waveform(path, asset, cancel).map(|w| w.to_bytes()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

/// See the module docs.
pub struct Previews {
    /// Swapped when the cache folder preference changes or the cache is cleared.
    cache: RwLock<Option<Arc<PreviewCache>>>,
    generate: Box<Generator>,
    inflight: Mutex<HashMap<Key, Vec<Waiter>>>,
    failed: Mutex<HashSet<Key>>,
}

impl std::fmt::Debug for Previews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Previews")
            .field("cache", &self.cache())
            .finish_non_exhaustive()
    }
}

/// A flag that is never set, for work nobody can cancel.
static NEVER: AtomicBool = AtomicBool::new(false);

impl Previews {
    /// With the engine's generators. Without a cache every request generates again.
    pub fn new(cache: Option<PreviewCache>) -> Self {
        Self::with_generator(cache, Box::new(engine_generator))
    }

    pub fn with_generator(cache: Option<PreviewCache>, generate: Box<Generator>) -> Self {
        Self {
            cache: RwLock::new(cache.map(Arc::new)),
            generate,
            inflight: Mutex::default(),
            failed: Mutex::default(),
        }
    }

    /// The cache in use now (a clone of the `Arc`, so no lock is held during file I/O).
    fn cache(&self) -> Option<Arc<PreviewCache>> {
        self.cache
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Moves the cache to `cache_root` (the effective cache folder from the preferences, design
    /// `preferences.md` §4), or reopens it there after **Clear cache**. Takes effect for the next
    /// request; the old folder is left as it is. Failures to remember are forgotten too, so a
    /// cleared cache retries everything.
    pub fn set_cache_root(&self, cache_root: &Path) {
        let cache = open_cache(cache_root).map(Arc::new);
        *self.cache.write().unwrap_or_else(PoisonError::into_inner) = cache;
        lock(&self.failed).clear();
    }

    /// The cached bytes of `key`.
    pub fn cached(&self, key: &Key) -> Option<Vec<u8>> {
        self.cache()?.get(&key.hash, key.product)
    }

    /// Cheap check for the prefetch plan: the file exists (it is not read).
    fn is_cached(&self, key: &Key) -> bool {
        self.cache()
            .and_then(|c| c.path(&key.hash, key.product))
            .is_some_and(|p| p.is_file())
    }

    pub fn has_failed(&self, key: &Key) -> bool {
        lock(&self.failed).contains(key)
    }

    /// Adds `waiter` for `key`. Returns `true` if nobody is producing `key` yet: the caller must
    /// then call [`Previews::produce`] for it (on a worker).
    pub fn attach(&self, key: Key, waiter: Waiter) -> bool {
        let mut inflight = lock(&self.inflight);
        match inflight.get_mut(&key) {
            Some(waiters) => {
                waiters.push(waiter);
                false
            }
            None => {
                inflight.insert(key, vec![waiter]);
                true
            }
        }
    }

    /// Produces `key` (from the cache if another producer just wrote it), stores it and answers
    /// every waiter attached to it. Call only after [`Previews::attach`] returned `true`.
    pub fn produce(&self, key: &Key, asset: &MediaAsset, cancel: &AtomicBool) {
        let outcome = self.make(key, asset, cancel);
        let waiters = lock(&self.inflight).remove(key).unwrap_or_default();
        for waiter in waiters {
            waiter(&outcome);
        }
    }

    fn make(&self, key: &Key, asset: &MediaAsset, cancel: &AtomicBool) -> Outcome {
        if let Some(bytes) = self.cached(key) {
            return Outcome::Ready(bytes.into());
        }
        let path = PathBuf::from(&asset.path.absolute);
        let generated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.generate)(&path, asset, key.product, cancel)
        }))
        .unwrap_or_else(|_| {
            Err(MediaError::new(
                MediaErrorKind::Damaged,
                "the preview generator panicked",
            ))
        });
        match generated {
            Ok(bytes) => {
                if let Some(cache) = self.cache()
                    && let Err(err) = cache.put(&key.hash, key.product, &bytes)
                {
                    tracing::warn!(%err, "could not cache a preview");
                }
                Outcome::Ready(bytes.into())
            }
            Err(err)
                if matches!(
                    err.kind,
                    MediaErrorKind::FileNotFound
                        | MediaErrorKind::AccessDenied
                        | MediaErrorKind::Cancelled
                ) =>
            {
                tracing::debug!(%err, product = ?key.product, "preview unavailable");
                Outcome::Unavailable
            }
            Err(err) => {
                tracing::warn!(%err, product = ?key.product, path = %path.display(), "preview generation failed");
                lock(&self.failed).insert(key.clone());
                Outcome::Failed
            }
        }
    }

    /// Generates `key` for an interactive request: answers `waiter` from the shared generation.
    pub fn request(
        self: &Arc<Self>,
        spawner: &JobSpawner,
        asset: MediaAsset,
        product: Product,
        waiter: Waiter,
    ) {
        let key = Key::new(&asset, product);
        if self.attach(key.clone(), waiter) {
            let previews = Arc::clone(self);
            spawner.run(Priority::Interactive, move || {
                previews.produce(&key, &asset, &NEVER);
            });
        }
    }

    /// Startup housekeeping: deletes old cache versions, then evicts.
    pub fn clean_up(&self) {
        if let Some(cache) = self.cache() {
            cache.remove_old_versions();
        }
        self.evict();
    }

    /// Deletes least recently used cache folders until the cache is under its cap.
    pub fn evict(&self) {
        if let Some(cache) = self.cache() {
            match cache.evict() {
                Ok(0) => {}
                Ok(n) => tracing::info!(folders = n, "evicted cached previews"),
                Err(err) => tracing::warn!(%err, "preview cache eviction failed"),
            }
        }
    }
}

/// Subfolder of the cache root (preferences §4) that holds the media previews.
pub const MEDIA_CACHE_SUBFOLDER: &str = "media";

/// Opens the preview cache at `<cache_root>\media\v1`, where `cache_root` is the effective cache
/// folder from the preferences (`<cacheDir or app cache dir>\cutframe-cache`). Without it previews
/// are generated on every request.
pub fn open_cache(cache_root: &Path) -> Option<PreviewCache> {
    let root = cache_root.join(MEDIA_CACHE_SUBFOLDER);
    match PreviewCache::open(&root, engine::media::cache::DEFAULT_CACHE_CAP) {
        Ok(cache) => {
            tracing::info!(dir = %cache.dir().display(), "preview cache");
            Some(cache)
        }
        Err(err) => {
            tracing::error!(%err, root = %root.display(), "cannot open the preview cache");
            None
        }
    }
}

/// Starts the background preview job (stage `generatingPreviews`) for `ids`, or for every asset
/// of the project when `ids` is `None` (after open). Generates, in order, all posters, then
/// waveforms, then filmstrip tiles that are not cached yet; ids no longer in the project when a
/// task runs (an undone import) are skipped. Evicts the cache at the end.
pub fn start_prefetch(
    jobs: &JobManager,
    previews: Arc<Previews>,
    ids: Option<Vec<MediaId>>,
) -> JobId {
    jobs.start(
        JobStage::GeneratingPreviews,
        Priority::Background,
        move |ctx| {
            plan(ctx, previews, ids);
        },
    )
}

fn plan(ctx: JobContext, previews: Arc<Previews>, ids: Option<Vec<MediaId>>) {
    let project = ctx.host().snapshot();
    let assets: Vec<&MediaAsset> = match &ids {
        Some(ids) => ids
            .iter()
            .filter_map(|id| project.media.iter().find(|m| m.id == *id))
            .collect(),
        None => project.media.iter().collect(),
    };
    let (mut posters, mut waveforms, mut tiles) = (Vec::new(), Vec::new(), Vec::new());
    for asset in assets {
        let (poster, waveform, filmstrip) = products(asset);
        posters.extend(poster.map(|p| (asset.id, p)));
        waveforms.extend(waveform.map(|p| (asset.id, p)));
        tiles.extend(filmstrip.into_iter().map(|p| (asset.id, p)));
    }
    let todo: Vec<(MediaId, Product)> = posters
        .into_iter()
        .chain(waveforms)
        .chain(tiles)
        .filter(|&(id, product)| {
            let asset = project
                .media
                .iter()
                .find(|m| m.id == id)
                .expect("planned from it");
            let key = Key::new(asset, product);
            !previews.is_cached(&key) && !previews.has_failed(&key)
        })
        .collect();
    ctx.set_total(todo.len());
    if todo.is_empty() {
        ctx.finish(Ok(()));
        return;
    }
    let remaining = Arc::new(AtomicUsize::new(todo.len()));
    for (id, product) in todo {
        let (previews, remaining) = (Arc::clone(&previews), Arc::clone(&remaining));
        ctx.spawn(Priority::Background, move |ctx| {
            prefetch_one(&ctx, &previews, id, product);
            ctx.step_done();
            if remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
                previews.evict();
                let result = if ctx.is_cancelled() {
                    Err(IpcError::Cancelled("preview generation cancelled".into()))
                } else {
                    Ok(())
                };
                ctx.finish(result);
            }
        });
    }
}

fn prefetch_one(ctx: &JobContext, previews: &Previews, id: MediaId, product: Product) {
    if ctx.is_cancelled() {
        return;
    }
    // The latest project: an import undone meanwhile has nothing left to generate.
    let project: Arc<Project> = ctx.host().snapshot();
    let Some(asset) = project.media.iter().find(|m| m.id == id) else {
        return;
    };
    let key = Key::new(asset, product);
    if previews.has_failed(&key) || previews.is_cached(&key) {
        return;
    }
    // Someone else (a `media://` request) is producing it already: nothing to wait for here.
    if previews.attach(key.clone(), Box::new(|_| {})) {
        previews.produce(&key, asset, ctx.cancel_flag());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use app_core::command::AddMedia;
    use app_core::media::samples;

    use super::*;
    use crate::jobs::tests::{FakeHost, Recorded, wait_finished};

    /// Counts generations per product and answers with `<product>@<hash>` bytes. Paths starting
    /// with `missing` fail as not found, `broken` as damaged.
    pub(crate) fn fake_generator(
        calls: Arc<Mutex<Vec<Product>>>,
        delay: Duration,
    ) -> Box<Generator> {
        Box::new(move |path, asset, product, _cancel| {
            calls.lock().unwrap().push(product);
            std::thread::sleep(delay);
            let name = path.to_string_lossy();
            if name.contains("missing") {
                return Err(MediaError::new(MediaErrorKind::FileNotFound, "gone"));
            }
            if name.contains("broken") {
                return Err(MediaError::new(MediaErrorKind::Damaged, "bad"));
            }
            Ok(format!("{product:?}@{}", asset.fingerprint.hash).into_bytes())
        })
    }

    pub(crate) fn temp_cache(name: &str) -> (PathBuf, PreviewCache) {
        let root = std::env::temp_dir().join(format!(
            "cutframe-previews-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&root).ok();
        let cache = PreviewCache::open(&root, engine::media::cache::DEFAULT_CACHE_CAP).unwrap();
        (root, cache)
    }

    fn channel_waiter() -> (Waiter, mpsc::Receiver<Outcome>) {
        let (tx, rx) = mpsc::channel();
        (
            Box::new(move |outcome: &Outcome| {
                tx.send(outcome.clone()).ok();
            }),
            rx,
        )
    }

    #[test]
    fn concurrent_requests_share_one_generation() {
        let calls = Arc::default();
        let (root, cache) = temp_cache("share");
        let previews = Arc::new(Previews::with_generator(
            Some(cache),
            fake_generator(Arc::clone(&calls), Duration::from_millis(200)),
        ));
        let (host, _rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 2);
        let asset = samples::video(1);
        let (first, first_rx) = channel_waiter();
        let (second, second_rx) = channel_waiter();
        previews.request(&jobs.spawner(), asset.clone(), Product::Poster, first);
        previews.request(&jobs.spawner(), asset.clone(), Product::Poster, second);
        let timeout = Duration::from_secs(10);
        let a = first_rx.recv_timeout(timeout).unwrap();
        let b = second_rx.recv_timeout(timeout).unwrap();
        assert_eq!(a, b);
        assert!(matches!(a, Outcome::Ready(_)));
        assert_eq!(*calls.lock().unwrap(), [Product::Poster], "one generation");

        // Cached now: answered from disk, nothing generated.
        assert!(
            previews
                .cached(&Key::new(&asset, Product::Poster))
                .is_some()
        );
        let (third, third_rx) = channel_waiter();
        previews.request(&jobs.spawner(), asset, Product::Poster, third);
        assert_eq!(third_rx.recv_timeout(timeout).unwrap(), a);
        assert_eq!(calls.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn failures_are_remembered_and_unreachable_files_are_not() {
        let calls = Arc::default();
        let previews =
            Previews::with_generator(None, fake_generator(Arc::clone(&calls), Duration::ZERO));
        let mut broken = samples::video(1);
        broken.path.absolute = "C:\\broken.mp4".into();
        let key = Key::new(&broken, Product::Poster);
        let (waiter, rx) = channel_waiter();
        assert!(previews.attach(key.clone(), waiter));
        previews.produce(&key, &broken, &NEVER);
        assert_eq!(rx.recv().unwrap(), Outcome::Failed);
        assert!(previews.has_failed(&key));

        let mut missing = samples::video(2);
        missing.path.absolute = "C:\\missing.mp4".into();
        let key = Key::new(&missing, Product::Poster);
        let (waiter, rx) = channel_waiter();
        assert!(previews.attach(key.clone(), waiter));
        previews.produce(&key, &missing, &NEVER);
        assert_eq!(rx.recv().unwrap(), Outcome::Unavailable);
        assert!(!previews.has_failed(&key), "retried on the next request");
    }

    #[test]
    fn the_cache_follows_the_cache_folder() {
        let calls = Arc::default();
        let (first, cache) = temp_cache("root-a");
        let previews = Previews::with_generator(
            Some(cache),
            fake_generator(Arc::clone(&calls), Duration::ZERO),
        );
        let asset = samples::video(1);
        let key = Key::new(&asset, Product::Poster);
        assert!(previews.attach(key.clone(), Box::new(|_| {})));
        previews.produce(&key, &asset, &NEVER);
        assert!(previews.cached(&key).is_some());

        // Another cache folder (preference change): empty, under `<root>/media/v1`.
        let second = first.with_extension("moved");
        std::fs::remove_dir_all(&second).ok();
        previews.set_cache_root(&second);
        assert!(previews.cached(&key).is_none());
        assert!(second.join(MEDIA_CACHE_SUBFOLDER).join("v1").is_dir());
        assert!(previews.attach(key.clone(), Box::new(|_| {})));
        previews.produce(&key, &asset, &NEVER);
        assert!(previews.cached(&key).is_some());
        assert!(first.join("v1").is_dir(), "the old folder is left as it is");
        std::fs::remove_dir_all(first).ok();
        std::fs::remove_dir_all(second).ok();
    }

    #[test]
    fn products_follow_the_kind_and_the_filmstrip_length() {
        let video = samples::video(1); // 5 s: one tile
        assert_eq!(
            products(&video),
            (
                Some(Product::Poster),
                Some(Product::Waveform),
                vec![Product::FilmstripTile(0)]
            )
        );
        assert!(has_product(&video, Product::FilmstripTile(0)));
        assert!(!has_product(&video, Product::FilmstripTile(1)));
        let audio = samples::audio(2);
        assert_eq!(products(&audio), (None, Some(Product::Waveform), vec![]));
        let image = samples::image(3);
        assert_eq!(products(&image), (Some(Product::Poster), None, vec![]));
        assert_eq!(preview_version(&video), "0101010101010101");
    }

    #[test]
    fn the_prefetch_job_generates_posters_first_and_skips_what_is_cached() {
        let calls = Arc::default();
        let (root, cache) = temp_cache("prefetch");
        let previews = Arc::new(Previews::with_generator(
            Some(cache),
            fake_generator(Arc::clone(&calls), Duration::ZERO),
        ));
        let (host, rx) = FakeHost::new();
        let assets = vec![samples::video(1), samples::audio(2), samples::image(3)];
        host.document
            .lock()
            .unwrap()
            .execute(Box::new(AddMedia::new(assets.clone())))
            .unwrap();
        // One worker, so the order is the queue order.
        let jobs = JobManager::with_workers(host.clone(), 1);
        let id = start_prefetch(&jobs, Arc::clone(&previews), None);
        assert_eq!(wait_finished(&rx), id);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                Product::Poster,
                Product::Poster,
                Product::Waveform,
                Product::Waveform,
                Product::FilmstripTile(0)
            ]
        );
        let finished: Vec<_> = host
            .events()
            .into_iter()
            .filter_map(|e| match e {
                Recorded::Finished(f) => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].result, Ok(()));

        // Everything is cached: a second run (a restart) generates nothing.
        let id = start_prefetch(&jobs, Arc::clone(&previews), Some(vec![assets[0].id]));
        assert_eq!(wait_finished(&rx), id);
        assert_eq!(calls.lock().unwrap().len(), 5);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_prefetch_job_skips_media_removed_meanwhile() {
        let calls = Arc::default();
        let previews = Arc::new(Previews::with_generator(
            None,
            fake_generator(Arc::clone(&calls), Duration::ZERO),
        ));
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        // The id is not (or no longer) in the project.
        let id = start_prefetch(&jobs, previews, Some(vec![MediaId::new()]));
        assert_eq!(wait_finished(&rx), id);
        assert!(calls.lock().unwrap().is_empty());
    }
}
