//! The content-keyed preview cache on disk (design §6.4).
//!
//! ```text
//! <root>\v1\9f\9f2c41d0e8a7b6c5d4e3f2a1b0c9d8e7\poster.jpg
//!                                              filmstrip-0000.jpg ...
//!                                              waveform.bin
//!                                              used     (empty; mtime = last use)
//! ```
//!
//! - Keyed by the fingerprint hash, so a moved file keeps its previews and a changed file gets new
//!   ones. The folder layout is the index; there is no database.
//! - Files are written as a unique `.tmp` and renamed, so a half-written file is never served.
//! - [`CACHE_VERSION`] changes when an output format or size changes; other version folders are
//!   deleted at startup ([`PreviewCache::remove_old_versions`]).
//! - [`PreviewCache::evict`] deletes whole asset folders, least recently used first, until the
//!   cache is under its cap.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// The current layout and format version; the folder under the cache root.
pub const CACHE_VERSION: &str = "v1";
/// Default size cap (task 0150 makes it configurable).
pub const DEFAULT_CACHE_CAP: u64 = 2 * 1024 * 1024 * 1024;
const USED_FILE: &str = "used";

/// One cached preview of an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    Poster,
    FilmstripTile(u32),
    Waveform,
}

impl Product {
    /// The file name inside the asset folder.
    pub fn file_name(self) -> String {
        match self {
            Self::Poster => "poster.jpg".to_owned(),
            Self::FilmstripTile(n) => format!("filmstrip-{n:04}.jpg"),
            Self::Waveform => "waveform.bin".to_owned(),
        }
    }

    /// The HTTP media type of the file.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Poster | Self::FilmstripTile(_) => "image/jpeg",
            Self::Waveform => "application/octet-stream",
        }
    }
}

/// True for a fingerprint hash as the model stores it: 32 lowercase hex characters. Anything else
/// would let a crafted hash escape the cache folder.
fn valid_hash(hash: &str) -> bool {
    hash.len() == 32 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The cache under a root folder (the shell passes `app_local_data_dir()\cache\media`).
#[derive(Debug)]
pub struct PreviewCache {
    /// `<root>\v1`.
    dir: PathBuf,
    cap: u64,
    /// Asset folders whose `used` file was touched in this session.
    touched: Mutex<HashSet<String>>,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl PreviewCache {
    /// Opens (creates) the cache under `root`. Call [`PreviewCache::remove_old_versions`] once,
    /// off the UI thread.
    pub fn open(root: &Path, cap: u64) -> std::io::Result<Self> {
        let dir = root.join(CACHE_VERSION);
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            cap,
            touched: Mutex::default(),
        })
    }

    /// Deletes the folders of other cache versions (`v0`, `v2`, ...) next to this one.
    pub fn remove_old_versions(&self) {
        let Some(root) = self.dir.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_version = name.len() > 1
                && name.starts_with('v')
                && name[1..].bytes().all(|b| b.is_ascii_digit());
            if is_version && name != CACHE_VERSION && entry.path().is_dir() {
                match std::fs::remove_dir_all(entry.path()) {
                    Ok(()) => {
                        tracing::info!(folder = %name, "deleted an old preview cache version")
                    }
                    Err(err) => {
                        tracing::warn!(%err, folder = %name, "could not delete an old preview cache")
                    }
                }
            }
        }
    }

    /// `<root>\v1`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn asset_dir(&self, hash: &str) -> Option<PathBuf> {
        valid_hash(hash).then(|| self.dir.join(&hash[..2]).join(hash))
    }

    /// The path of a product's file (it may not exist).
    pub fn path(&self, hash: &str, product: Product) -> Option<PathBuf> {
        self.asset_dir(hash)
            .map(|dir| dir.join(product.file_name()))
    }

    /// The cached bytes, or `None` if they are not cached (or the hash is malformed). Marks the
    /// asset as used, at most once per session.
    pub fn get(&self, hash: &str, product: Product) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path(hash, product)?).ok()?;
        self.touch(hash);
        Some(bytes)
    }

    /// Writes `bytes` atomically (unique temporary file, then rename) and marks the asset used.
    pub fn put(&self, hash: &str, product: Product, bytes: &[u8]) -> std::io::Result<()> {
        let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "malformed hash");
        let dir = self.asset_dir(hash).ok_or_else(invalid)?;
        std::fs::create_dir_all(&dir)?;
        let target = dir.join(product.file_name());
        let tmp = dir.join(format!(
            "{}.{}-{}.tmp",
            product.file_name(),
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, &target)
        })();
        if written.is_err() {
            std::fs::remove_file(&tmp).ok();
        }
        written?;
        self.touch(hash);
        Ok(())
    }

    /// Sets the asset's `used` mtime to now, once per session.
    fn touch(&self, hash: &str) {
        let first = self
            .touched
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(hash.to_owned());
        if !first {
            return;
        }
        let Some(dir) = self.asset_dir(hash) else {
            return;
        };
        let used = dir.join(USED_FILE);
        let result = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&used)
            .and_then(|file| file.set_modified(SystemTime::now()));
        if let Err(err) = result {
            tracing::debug!(%err, "could not mark a cached preview as used");
        }
    }

    /// Deletes whole asset folders, oldest `used` first, until the cache is at most its cap.
    /// Leftover `.tmp` files count toward the size and are deleted with their folder. Returns the
    /// number of folders deleted.
    pub fn evict(&self) -> std::io::Result<usize> {
        let mut assets: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        let mut total = 0u64;
        for shard in std::fs::read_dir(&self.dir)?.flatten() {
            if !shard.path().is_dir() {
                continue;
            }
            for asset in std::fs::read_dir(shard.path())?.flatten() {
                let path = asset.path();
                if !path.is_dir() {
                    continue;
                }
                let mut size = 0;
                let mut newest = SystemTime::UNIX_EPOCH;
                for file in std::fs::read_dir(&path)?.flatten() {
                    if let Ok(meta) = file.metadata() {
                        size += meta.len();
                        if let Ok(modified) = meta.modified() {
                            newest = newest.max(modified);
                        }
                    }
                }
                // `used` is the last use; folders without one fall back to their newest file.
                let used = std::fs::metadata(path.join(USED_FILE))
                    .and_then(|m| m.modified())
                    .unwrap_or(newest);
                total += size;
                assets.push((used, size, path));
            }
        }
        if total <= self.cap {
            return Ok(0);
        }
        assets.sort_by_key(|(used, _, path)| (*used, path.clone()));
        let mut deleted = 0;
        for (_, size, path) in assets {
            if total <= self.cap {
                break;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    total = total.saturating_sub(size);
                    deleted += 1;
                    if let Some(name) = path.file_name() {
                        self.touched
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .remove(&*name.to_string_lossy());
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "could not evict previews")
                }
            }
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn hash(seed: u8) -> String {
        format!("{seed:02x}").repeat(16)
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cutframe-cache-{name}-{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&root).ok();
        root
    }

    fn set_used(cache: &PreviewCache, hash: &str, age_secs: u64) {
        let used = cache.asset_dir(hash).unwrap().join(USED_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(used)
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
    }

    #[test]
    fn put_then_get_by_content_and_product() {
        let root = temp_root("put");
        let cache = PreviewCache::open(&root, DEFAULT_CACHE_CAP).unwrap();
        assert_eq!(cache.get(&hash(1), Product::Poster), None);
        cache.put(&hash(1), Product::Poster, b"jpeg").unwrap();
        cache
            .put(&hash(1), Product::FilmstripTile(3), b"tile")
            .unwrap();
        assert_eq!(
            cache.get(&hash(1), Product::Poster).as_deref(),
            Some(&b"jpeg"[..])
        );
        assert_eq!(
            cache.get(&hash(1), Product::FilmstripTile(3)).as_deref(),
            Some(&b"tile"[..])
        );
        assert_eq!(cache.get(&hash(1), Product::Waveform), None);
        // Another fingerprint (the file changed) has nothing cached.
        assert_eq!(cache.get(&hash(2), Product::Poster), None);

        let dir = root.join("v1").join("01").join(hash(1));
        assert!(dir.join("poster.jpg").is_file());
        assert!(dir.join("filmstrip-0003.jpg").is_file());
        assert!(dir.join("used").is_file());
        // No temporary file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_temporary_file_is_never_served() {
        let root = temp_root("tmp");
        let cache = PreviewCache::open(&root, DEFAULT_CACHE_CAP).unwrap();
        // A crash mid-write leaves only the temporary file.
        let dir = cache.asset_dir(&hash(5)).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("poster.jpg.123-0.tmp"), b"half").unwrap();
        assert_eq!(cache.get(&hash(5), Product::Poster), None);
        cache.put(&hash(5), Product::Poster, b"whole").unwrap();
        assert_eq!(
            cache.get(&hash(5), Product::Poster).as_deref(),
            Some(&b"whole"[..])
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn malformed_hashes_never_touch_the_disk() {
        let root = temp_root("hash");
        let cache = PreviewCache::open(&root, DEFAULT_CACHE_CAP).unwrap();
        for bad in ["", "../../etc/passwd", &"A".repeat(32), &"0".repeat(31)] {
            assert!(cache.path(bad, Product::Poster).is_none());
            assert!(cache.put(bad, Product::Poster, b"x").is_err());
            assert_eq!(cache.get(bad, Product::Poster), None);
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn other_version_folders_are_deleted() {
        let root = temp_root("versions");
        std::fs::create_dir_all(root.join("v0").join("ab")).unwrap();
        std::fs::create_dir_all(root.join("v7")).unwrap();
        std::fs::create_dir_all(root.join("keep-me")).unwrap();
        let cache = PreviewCache::open(&root, DEFAULT_CACHE_CAP).unwrap();
        assert!(cache.dir().is_dir());
        assert!(root.join("v0").exists(), "not on open");
        cache.remove_old_versions();
        assert!(cache.dir().is_dir());
        assert!(!root.join("v0").exists());
        assert!(!root.join("v7").exists());
        assert!(
            root.join("keep-me").exists(),
            "only version folders are deleted"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn eviction_deletes_least_recently_used_assets_first() {
        let root = temp_root("evict");
        // Room for two 100-byte assets.
        let cache = PreviewCache::open(&root, 250).unwrap();
        for seed in 1..=4 {
            cache.put(&hash(seed), Product::Poster, &[0; 100]).unwrap();
        }
        // Oldest use: 3, then 1, then 4, then 2.
        set_used(&cache, &hash(3), 400);
        set_used(&cache, &hash(1), 300);
        set_used(&cache, &hash(4), 200);
        set_used(&cache, &hash(2), 100);

        assert_eq!(cache.evict().unwrap(), 2);
        assert_eq!(cache.get(&hash(3), Product::Poster), None);
        assert_eq!(cache.get(&hash(1), Product::Poster), None);
        assert!(cache.get(&hash(4), Product::Poster).is_some());
        assert!(cache.get(&hash(2), Product::Poster).is_some());
        // Under the cap: nothing more to do.
        assert_eq!(cache.evict().unwrap(), 0);
        std::fs::remove_dir_all(root).ok();
    }
}
