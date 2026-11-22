//! Media status: whether each media file is reachable and unchanged (design `media-import.md`
//! §6.6). Session state in `AppState`, never in the model or the file, and no history.
//!
//! - A status belongs to an asset's **(path, content hash)**, not to its id. Undoing a relink puts
//!   the old path back, and with it the old status, without any bookkeeping. A relink to a new
//!   path starts `online`.
//! - Before a check finishes, everything counts as `online`. Opening or restoring a project
//!   clears the statuses and starts a background `checkingMedia` job: missing → `offline`; size
//!   and mtime equal to the fingerprint → `online`; otherwise re-hash: same hash → `online`,
//!   different → `changed`.
//! - At runtime a preview generator that finds the file gone reports it `offline`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use app_core::MediaAsset;
use engine::media::{MediaErrorKind, fingerprint, modified_millis};

use crate::ipc::IpcError;
use crate::ipc::types::{JobId, JobStage, MediaStatus};
use crate::jobs::{JobContext, JobManager, Priority};
use crate::media_paths::path_key;

/// What a status is about: a file path (case-folded on Windows) and the content expected there.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatusKey {
    path: String,
    hash: String,
}

impl StatusKey {
    pub fn of(asset: &MediaAsset) -> Self {
        Self {
            path: path_key(Path::new(&asset.path.absolute)),
            hash: asset.fingerprint.hash.clone(),
        }
    }
}

pub type StatusUpdate = (StatusKey, MediaStatus);

/// The statuses of this session; absent means `online`.
#[derive(Debug, Default, Clone)]
pub struct MediaStatuses(HashMap<StatusKey, MediaStatus>);

impl MediaStatuses {
    pub fn get(&self, asset: &MediaAsset) -> MediaStatus {
        self.0
            .get(&StatusKey::of(asset))
            .copied()
            .unwrap_or(MediaStatus::Online)
    }

    /// Records `updates`. Returns whether any status changed.
    pub fn apply(&mut self, updates: Vec<StatusUpdate>) -> bool {
        let mut changed = false;
        for (key, status) in updates {
            let previous = if status == MediaStatus::Online {
                self.0.remove(&key)
            } else {
                self.0.insert(key, status)
            };
            changed |= previous.unwrap_or(MediaStatus::Online) != status;
        }
        changed
    }

    /// Forgets everything (another project was opened): all media counts as online again.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// What the file at `asset`'s path is now. Reads at most 2 MiB (the fingerprint), and only when
/// size or mtime differ from the stored fingerprint.
pub fn check(asset: &MediaAsset) -> MediaStatus {
    let path = Path::new(&asset.path.absolute);
    let meta = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        // Gone, not a file any more, or the drive or share cannot be reached.
        _ => return MediaStatus::Offline,
    };
    let stored = &asset.fingerprint;
    if meta.len() != stored.size {
        // The size is part of the hash: other content for sure.
        return MediaStatus::Changed;
    }
    if modified_millis(&meta) == stored.modified {
        return MediaStatus::Online;
    }
    match fingerprint(path) {
        Ok(now) if now.hash == stored.hash => MediaStatus::Online,
        Ok(_) => MediaStatus::Changed,
        Err(err) if err.kind == MediaErrorKind::FileNotFound => MediaStatus::Offline,
        // Locked or unreadable right now: not known to be missing or changed.
        Err(err) => {
            tracing::info!(%err, media = %asset.id, "media check could not read the file");
            MediaStatus::Online
        }
    }
}

/// Starts the background `checkingMedia` job over every asset of the open project. Its result
/// arrives as one status update (one `project-changed` if anything is not online). A cancelled
/// check records nothing.
pub fn start_check(jobs: &JobManager) -> JobId {
    jobs.start(JobStage::CheckingMedia, Priority::Background, plan)
}

fn plan(ctx: JobContext) {
    let project = ctx.host().snapshot();
    let assets: Arc<[MediaAsset]> = project.media.iter().cloned().collect();
    ctx.set_total(assets.len());
    if assets.is_empty() {
        ctx.finish(Ok(()));
        return;
    }
    let results: Arc<Mutex<Vec<Option<MediaStatus>>>> =
        Arc::new(Mutex::new(vec![None; assets.len()]));
    let remaining = Arc::new(AtomicUsize::new(assets.len()));
    for index in 0..assets.len() {
        let (assets, results, remaining) = (
            Arc::clone(&assets),
            Arc::clone(&results),
            Arc::clone(&remaining),
        );
        ctx.spawn(Priority::Background, move |ctx| {
            if !ctx.is_cancelled() {
                let status = std::panic::catch_unwind(|| check(&assets[index])).ok();
                results.lock().unwrap_or_else(|p| p.into_inner())[index] = status;
            }
            ctx.step_done();
            if remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
                if ctx.is_cancelled() {
                    ctx.finish(Err(IpcError::Cancelled("media check cancelled".into())));
                    return;
                }
                let results =
                    std::mem::take(&mut *results.lock().unwrap_or_else(|p| p.into_inner()));
                let updates = assets
                    .iter()
                    .zip(results)
                    .filter_map(|(asset, status)| Some((StatusKey::of(asset), status?)))
                    .collect();
                ctx.host().media_status(updates);
                ctx.finish(Ok(()));
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use app_core::command::AddMedia;
    use app_core::media::{MediaPath, samples};

    use super::*;
    use crate::jobs::tests::{FakeHost, Recorded, wait_finished};

    /// A sample video asset whose path and fingerprint are those of the file at `path`.
    pub(crate) fn asset_for(path: &Path, seed: u8) -> MediaAsset {
        let mut asset = samples::video(seed);
        asset.path = MediaPath::absolute(path.to_string_lossy());
        asset.fingerprint = fingerprint(path).expect("fingerprint");
        asset
    }

    fn set_mtime(path: &Path, time: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn statuses_belong_to_path_and_content() {
        let mut statuses = MediaStatuses::default();
        let asset = samples::video(1);
        assert_eq!(
            statuses.get(&asset),
            MediaStatus::Online,
            "online by default"
        );
        assert!(statuses.apply(vec![(StatusKey::of(&asset), MediaStatus::Offline)]));
        assert!(!statuses.apply(vec![(StatusKey::of(&asset), MediaStatus::Offline)]));
        assert_eq!(statuses.get(&asset), MediaStatus::Offline);

        let mut relinked = asset.clone();
        relinked.path = MediaPath::absolute("D:\\new\\a.mp4");
        assert_eq!(statuses.get(&relinked), MediaStatus::Online);
        let mut changed = asset.clone();
        changed.fingerprint.hash = "ab".repeat(16);
        assert_eq!(statuses.get(&changed), MediaStatus::Online);

        assert!(statuses.apply(vec![(StatusKey::of(&asset), MediaStatus::Online)]));
        assert_eq!(statuses.get(&asset), MediaStatus::Online);
        assert!(!statuses.apply(vec![(StatusKey::of(&asset), MediaStatus::Online)]));
    }

    #[test]
    fn check_tells_online_changed_and_offline_apart() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| -> PathBuf { dir.path().join(name) };
        for name in [
            "same.mp4",
            "touched.mp4",
            "edited.mp4",
            "grown.mp4",
            "gone.mp4",
        ] {
            fs::write(path(name), format!("content of {name}")).unwrap();
        }
        let assets: Vec<_> = [
            "same.mp4",
            "touched.mp4",
            "edited.mp4",
            "grown.mp4",
            "gone.mp4",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| asset_for(&path(n), i as u8 + 1))
        .collect();

        let later = SystemTime::now() + Duration::from_secs(120);
        set_mtime(&path("touched.mp4"), later);
        // Same size, other bytes, new mtime: only the hash can tell.
        fs::write(path("edited.mp4"), "CONTENT OF edited.mp4").unwrap();
        set_mtime(&path("edited.mp4"), later);
        fs::write(path("grown.mp4"), "more content of grown.mp4").unwrap();
        fs::remove_file(path("gone.mp4")).unwrap();

        let statuses: Vec<_> = assets.iter().map(check).collect();
        use MediaStatus::*;
        assert_eq!(statuses, [Online, Online, Changed, Changed, Offline]);

        let mut folder = assets[0].clone();
        folder.path = MediaPath::absolute(dir.path().to_string_lossy());
        assert_eq!(check(&folder), Offline, "a folder is not the file");
    }

    #[test]
    fn the_check_job_records_every_status_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let (host, rx) = FakeHost::new();
        let a = dir.path().join("a.mp4");
        let b = dir.path().join("b.mp4");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let assets = vec![asset_for(&a, 1), asset_for(&b, 2)];
        host.document
            .lock()
            .unwrap()
            .execute(Box::new(AddMedia::new(assets.clone())))
            .unwrap();
        fs::remove_file(&b).unwrap();

        let jobs = JobManager::with_workers(host.clone(), 2);
        let id = start_check(&jobs);
        assert_eq!(wait_finished(&rx), id);
        assert!(host.events().iter().any(|e| matches!(
            e,
            Recorded::Finished(f) if f.job_id == id && f.result.is_ok()
        )));
        let statuses = host.statuses.lock().unwrap();
        assert_eq!(statuses.get(&assets[0]), MediaStatus::Online);
        assert_eq!(statuses.get(&assets[1]), MediaStatus::Offline);
        assert!(
            host.events().iter().all(|e| match e {
                Recorded::Progress(p) => p.stage == JobStage::CheckingMedia,
                _ => true,
            }),
            "progress under the checkingMedia stage"
        );
    }

    #[test]
    fn checking_an_empty_project_finishes_at_once() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 1);
        let id = start_check(&jobs);
        assert_eq!(wait_finished(&rx), id);
    }
}
