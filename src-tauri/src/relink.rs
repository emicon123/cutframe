//! Relink jobs (design `media-import.md` §6.6): point media at the file where it lives now.
//!
//! - **One file** (`relink_media`): fingerprint and probe the chosen file. Another kind →
//!   `failed` with `kindMismatch`. Another hash → `fingerprintMismatch` unless `allow_mismatch`
//!   (the UI asks, then calls again with it; also "use the file as it is now" for `changed`
//!   media). Otherwise one `RelinkMedia` (history entry `relinkMedia`) → `relinked`.
//! - **A folder** (`relink_media_folder`): every asset whose file is missing is searched in the
//!   folder (recursively) by content only: files are indexed by size, and only those whose size
//!   matches are hashed, same file name first. All hash matches are relinked in one `Batch` (one
//!   undo step, `relinkMedia`); the rest are reported `notFound`. A file with the right name but
//!   other content is never taken.
//! - The report (`media-job-report`) comes just before `job-finished`; a relinked asset's
//!   previews are prefetched afterwards. While a gesture is open the edit waits for it to end.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use app_core::command::{Batch, Command, CommandError, HistoryLabel, RelinkMedia};
use app_core::media::{Fingerprint, MediaPath, validate_asset};
use app_core::{MediaAsset, MediaId};
use engine::media::fingerprint;

use crate::import::{FileResult, probe_file, walk};
use crate::ipc::IpcError;
use crate::ipc::events::MediaJobReport;
use crate::ipc::types::{
    JobId, JobStage, MediaErrorKind, MediaFileOutcome, MediaFileResult, MediaStatus,
};
use crate::jobs::{JobContext, JobManager, Priority, edit_when_idle};
use crate::media_paths::normalise;
use crate::media_status::StatusKey;

/// Starts relinking media `id` to the file at `path`. Returns at once; stage `probing`.
pub fn start_one(jobs: &JobManager, id: MediaId, path: PathBuf, allow_mismatch: bool) -> JobId {
    jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
        relink_one(ctx, id, &path, allow_mismatch);
    })
}

/// Starts searching `folder` for every missing media file. Returns at once; stage
/// `searchingFolder`.
pub fn start_folder(jobs: &JobManager, folder: PathBuf) -> JobId {
    jobs.start(
        JobStage::SearchingFolder,
        Priority::Interactive,
        move |ctx| {
            relink_folder(ctx, &folder);
        },
    )
}

fn report(ctx: &JobContext, files: Vec<MediaFileResult>) {
    ctx.host().media_job_report(MediaJobReport {
        job_id: ctx.id(),
        files,
    });
}

fn cancelled(ctx: JobContext) {
    ctx.finish(Err(IpcError::Cancelled("relink cancelled".into())));
}

/// `asset` found at `path` with `fingerprint` (and, for other content, `info` from the probe).
fn moved_to(asset: &MediaAsset, path: String, fingerprint: Fingerprint) -> MediaAsset {
    MediaAsset {
        path: MediaPath::absolute(path),
        fingerprint,
        ..asset.clone()
    }
}

/// Applies `relinked` as one history entry once no gesture is open. Assets that are no longer in
/// the project, or were changed meanwhile (`expected` differs), are skipped. Returns the ids that
/// were relinked, or `None` if the job was cancelled while waiting.
fn apply(
    ctx: &JobContext,
    relinked: &[(MediaAsset, MediaAsset)],
) -> Result<Option<Vec<MediaId>>, IpcError> {
    edit_when_idle(ctx, |doc| {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        let mut ids = Vec::new();
        for (expected, asset) in relinked {
            let Some(now) = doc.project().media.iter().find(|m| m.id == asset.id) else {
                continue;
            };
            if now != expected {
                continue;
            }
            ids.push(asset.id);
            if now != asset {
                commands.push(Box::new(RelinkMedia::new(asset.clone())));
            }
        }
        let changed = !commands.is_empty();
        match commands.len() {
            0 => {}
            1 => doc.execute(commands.pop().expect("one command"))?,
            _ => doc.execute(Box::new(Batch::new(HistoryLabel::RelinkMedia, commands)))?,
        }
        Ok::<_, CommandError>((ids, changed))
    })
}

fn relink_one(ctx: JobContext, id: MediaId, path: &Path, allow_mismatch: bool) {
    let path = normalise(path);
    let shown = path.to_string_lossy().into_owned();
    let project = ctx.host().snapshot();
    let Some(current) = project.media.iter().find(|m| m.id == id).cloned() else {
        ctx.finish(Err(IpcError::InvalidEdit(format!(
            "media {id} is not in the project"
        ))));
        return;
    };
    let failed = |ctx: JobContext, error: MediaErrorKind, detail: String| {
        report(
            &ctx,
            vec![MediaFileResult {
                path: shown.clone(),
                outcome: MediaFileOutcome::Failed { error, detail },
            }],
        );
        ctx.finish(Ok(()));
    };
    if path.to_str().is_none() {
        failed(
            ctx,
            MediaErrorKind::UnsupportedPath,
            "not valid UTF-8".into(),
        );
        return;
    }
    let (fingerprint, probed) = match probe_file(&path, &ctx) {
        FileResult::Probed {
            fingerprint,
            probed,
        } => (fingerprint, probed),
        FileResult::Failed(err) => return failed(ctx, err.kind.into(), err.detail),
        FileResult::Cancelled => return cancelled(ctx),
    };
    if probed.kind != current.kind {
        let detail = format!("media is {:?}, the file is {:?}", current.kind, probed.kind);
        failed(ctx, MediaErrorKind::KindMismatch, detail);
        return;
    }
    let same_content = fingerprint.hash == current.fingerprint.hash;
    if !same_content && !allow_mismatch {
        report(
            &ctx,
            vec![MediaFileResult {
                path: shown,
                outcome: MediaFileOutcome::FingerprintMismatch { media: id },
            }],
        );
        ctx.finish(Ok(()));
        return;
    }
    let mut relinked = moved_to(&current, shown.clone(), fingerprint);
    if !same_content {
        relinked.info = probed.info;
    }
    if let Err(problem) = validate_asset(&relinked) {
        tracing::error!(path = %shown, %problem, "probe produced an invalid asset");
        failed(
            ctx,
            MediaErrorKind::Damaged,
            format!("invalid probe result: {problem}"),
        );
        return;
    }
    let relinked_ids = match apply(&ctx, &[(current, relinked.clone())]) {
        Ok(Some(ids)) => ids,
        Ok(None) => return cancelled(ctx),
        Err(err) => {
            ctx.finish(Err(err));
            return;
        }
    };
    if relinked_ids.is_empty() {
        // Removed or relinked elsewhere while the file was probed.
        ctx.finish(Err(IpcError::InvalidEdit(format!(
            "media {id} changed while it was relinked"
        ))));
        return;
    }
    // A relink to the same path and content (the file came back) changes no model data, only the
    // status.
    ctx.host()
        .media_status(vec![(StatusKey::of(&relinked), MediaStatus::Online)]);
    report(
        &ctx,
        vec![MediaFileResult {
            path: shown,
            outcome: MediaFileOutcome::Relinked { media: id },
        }],
    );
    ctx.host().media_added(vec![id]);
    ctx.finish(Ok(()));
}

/// Same file name, ignoring case (Windows file names do).
fn same_name(a: &Path, b: &str) -> bool {
    a.file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase() == b.to_lowercase())
}

fn relink_folder(ctx: JobContext, folder: &Path) {
    let folder = normalise(folder);
    if !folder.is_dir() {
        ctx.finish(Err(IpcError::FileNotFound(format!(
            "{} is not a folder",
            folder.display()
        ))));
        return;
    }
    let project = ctx.host().snapshot();
    let missing: Vec<MediaAsset> = project
        .media
        .iter()
        .filter(|m| !Path::new(&m.path.absolute).is_file())
        .cloned()
        .collect();
    ctx.set_total(missing.len());
    if missing.is_empty() {
        report(&ctx, Vec::new());
        ctx.finish(Ok(()));
        return;
    }
    let files = match walk(&folder, usize::MAX) {
        Ok(Ok(files)) => files,
        Ok(Err(())) => unreachable!("no limit"),
        Err(err) => {
            let message = format!("cannot read {}: {err}", folder.display());
            ctx.finish(Err(match err.kind() {
                std::io::ErrorKind::NotFound => IpcError::FileNotFound(message),
                _ => IpcError::Io(message),
            }));
            return;
        }
    };
    // Size → files, so only candidates whose size matches are ever read.
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for file in files {
        if file.to_str().is_none() {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&file) {
            by_size.entry(meta.len()).or_default().push(file);
        }
    }

    let mut hashed: HashMap<PathBuf, Option<Fingerprint>> = HashMap::new();
    let mut found: Vec<Option<(String, Fingerprint)>> = Vec::with_capacity(missing.len());
    for asset in &missing {
        if ctx.is_cancelled() {
            return cancelled(ctx);
        }
        let mut candidates: Vec<&PathBuf> = by_size
            .get(&asset.fingerprint.size)
            .map(|files| files.iter().collect())
            .unwrap_or_default();
        let name = asset.file_name();
        // Stable: same name first, the rest in path order.
        candidates.sort_by_key(|c| !same_name(c, name));
        let hit = candidates.into_iter().find_map(|candidate| {
            let print = hashed
                .entry(candidate.clone())
                .or_insert_with(|| fingerprint(candidate).ok());
            print
                .as_ref()
                .filter(|p| p.hash == asset.fingerprint.hash)
                .map(|p| (candidate.to_string_lossy().into_owned(), p.clone()))
        });
        found.push(hit);
        ctx.step_done();
    }
    if ctx.is_cancelled() {
        return cancelled(ctx);
    }

    let relinked: Vec<(MediaAsset, MediaAsset)> = missing
        .iter()
        .zip(&found)
        .filter_map(|(asset, hit)| {
            let (path, print) = hit.clone()?;
            Some((asset.clone(), moved_to(asset, path, print)))
        })
        .collect();
    let applied = if relinked.is_empty() {
        Vec::new()
    } else {
        match apply(&ctx, &relinked) {
            Ok(Some(ids)) => ids,
            Ok(None) => return cancelled(ctx),
            Err(err) => {
                ctx.finish(Err(err));
                return;
            }
        }
    };

    let mut updates = Vec::new();
    let mut files = Vec::new();
    for (asset, hit) in missing.iter().zip(found) {
        match hit {
            Some((path, _)) if applied.contains(&asset.id) => {
                let (_, now) = relinked
                    .iter()
                    .find(|(old, _)| old.id == asset.id)
                    .expect("applied ids come from it");
                updates.push((StatusKey::of(now), MediaStatus::Online));
                files.push(MediaFileResult {
                    path,
                    outcome: MediaFileOutcome::Relinked { media: asset.id },
                });
            }
            // Found, but the asset was removed or changed meanwhile: nothing to report.
            Some(_) => {}
            None => {
                updates.push((StatusKey::of(asset), MediaStatus::Offline));
                files.push(MediaFileResult {
                    path: asset.path.absolute.clone(),
                    outcome: MediaFileOutcome::NotFound { media: asset.id },
                });
            }
        }
    }
    ctx.host().media_status(updates);
    report(&ctx, files);
    if !applied.is_empty() {
        ctx.host().media_added(applied);
    }
    ctx.finish(Ok(()));
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, mpsc};

    use app_core::Project;

    use super::*;
    use crate::ipc::events::JobFinished;
    use crate::jobs::tests::{FakeHost, Recorded, wait_finished};
    use crate::media_status::start_check;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/engine/tests/fixtures/media")
            .join(name)
    }

    /// The report and the finish event of job `id`, checking that the report comes first.
    fn outcome(host: &FakeHost, id: JobId) -> (Vec<MediaFileResult>, JobFinished) {
        let events = host.events();
        let finished = events
            .iter()
            .position(|e| matches!(e, Recorded::Finished(f) if f.job_id == id))
            .expect("job-finished");
        let report = events[..finished].iter().find_map(|e| match e {
            Recorded::Report(r) if r.job_id == id => Some(r.files.clone()),
            _ => None,
        });
        let Recorded::Finished(done) = events[finished].clone() else {
            unreachable!()
        };
        (report.unwrap_or_default(), done)
    }

    struct Setup {
        host: Arc<FakeHost>,
        rx: mpsc::Receiver<JobId>,
        jobs: JobManager,
        dir: tempfile::TempDir,
    }

    fn setup() -> Setup {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 2);
        Setup {
            host,
            rx,
            jobs,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    impl Setup {
        fn path(&self, relative: &str) -> PathBuf {
            let path = self.dir.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            path
        }

        /// Copies fixtures to `(fixture, relative path)` and imports them; returns their ids.
        fn import(&self, files: &[(&str, &str)]) -> Vec<MediaId> {
            let paths: Vec<PathBuf> = files
                .iter()
                .map(|(name, to)| {
                    let path = self.path(to);
                    fs::copy(fixture(name), &path).unwrap();
                    path
                })
                .collect();
            let id = crate::import::start(&self.jobs, paths);
            assert_eq!(wait_finished(&self.rx), id);
            outcome(&self.host, id)
                .0
                .iter()
                .map(|f| match f.outcome {
                    MediaFileOutcome::Added { media } => media,
                    ref other => panic!("{other:?}"),
                })
                .collect()
        }

        fn run(&self, id: JobId) -> (Vec<MediaFileResult>, JobFinished) {
            assert_eq!(wait_finished(&self.rx), id);
            outcome(&self.host, id)
        }

        fn project(&self) -> Arc<Project> {
            self.host.document.lock().unwrap().snapshot()
        }

        fn asset(&self, id: MediaId) -> MediaAsset {
            self.project()
                .media
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .clone()
        }

        fn status(&self, asset: &MediaAsset) -> MediaStatus {
            self.host.statuses.lock().unwrap().get(asset)
        }

        fn changes(&self) -> usize {
            self.host.changes.load(Ordering::Relaxed)
        }
    }

    fn shown(path: &Path) -> String {
        normalise(path).to_string_lossy().into_owned()
    }

    #[test]
    fn a_moved_file_is_relinked_as_one_undo_step() {
        let s = setup();
        let [id] = s.import(&[("h264-aac.mp4", "old/clip.mp4")])[..] else {
            unreachable!()
        };
        let before = s.asset(id);
        let moved = s.path("new/renamed.mp4");
        fs::rename(s.path("old/clip.mp4"), &moved).unwrap();
        let changes = s.changes();

        let (files, done) = s.run(start_one(&s.jobs, id, moved.clone(), false));
        assert_eq!(done.result, Ok(()));
        assert_eq!(
            files,
            [MediaFileResult {
                path: shown(&moved),
                outcome: MediaFileOutcome::Relinked { media: id }
            }]
        );
        let after = s.asset(id);
        assert_eq!(Path::new(&after.path.absolute), normalise(&moved));
        assert_eq!(after.fingerprint.hash, before.fingerprint.hash);
        assert_eq!(after.info, before.info);
        assert_eq!(s.changes(), changes + 1);
        assert_ne!(
            crate::previews::preview_version(&after),
            crate::previews::preview_version(&before),
            "the UI fetches the previews again"
        );
        assert_eq!(s.host.added.lock().unwrap().last(), Some(&vec![id]));

        let mut doc = s.host.document.lock().unwrap();
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RelinkMedia));
        assert!(doc.undo());
        assert_eq!(doc.project().media[0], before);
    }

    #[test]
    fn other_content_needs_allow_mismatch_and_brings_its_own_info() {
        let s = setup();
        let [id] = s.import(&[("h264-aac.mp4", "clip.mp4")])[..] else {
            unreachable!()
        };
        let before = s.asset(id);
        let other = s.path("other.mov");
        fs::copy(fixture("hevc-portrait-cw90-vfr.mov"), &other).unwrap();
        let changes = s.changes();

        let (files, done) = s.run(start_one(&s.jobs, id, other.clone(), false));
        assert_eq!(done.result, Ok(()));
        assert_eq!(
            files[0].outcome,
            MediaFileOutcome::FingerprintMismatch { media: id }
        );
        assert_eq!(s.changes(), changes, "nothing changed");
        assert_eq!(s.asset(id), before);

        let (files, _) = s.run(start_one(&s.jobs, id, other.clone(), true));
        assert_eq!(files[0].outcome, MediaFileOutcome::Relinked { media: id });
        let after = s.asset(id);
        assert_ne!(after.fingerprint.hash, before.fingerprint.hash);
        assert_eq!(
            after.info.video.as_ref().unwrap().codec,
            "hevc",
            "probe info of the new file"
        );
        assert_eq!(after.kind, before.kind);
    }

    #[test]
    fn another_kind_or_a_missing_file_is_reported_and_changes_nothing() {
        let s = setup();
        let [id] = s.import(&[("h264-aac.mp4", "clip.mp4")])[..] else {
            unreachable!()
        };
        let audio = s.path("song.mp3");
        fs::copy(fixture("stereo-44k.mp3"), &audio).unwrap();
        let changes = s.changes();

        let (files, done) = s.run(start_one(&s.jobs, id, audio, true));
        assert_eq!(done.result, Ok(()));
        assert!(matches!(
            files[0].outcome,
            MediaFileOutcome::Failed {
                error: MediaErrorKind::KindMismatch,
                ..
            }
        ));

        let (files, _) = s.run(start_one(&s.jobs, id, s.path("nope.mp4"), true));
        assert!(matches!(
            files[0].outcome,
            MediaFileOutcome::Failed {
                error: MediaErrorKind::FileNotFound,
                ..
            }
        ));

        let (_, done) = s.run(start_one(&s.jobs, MediaId::new(), s.path("clip.mp4"), true));
        assert!(matches!(done.result, Err(IpcError::InvalidEdit(_))));
        assert_eq!(s.changes(), changes);
    }

    #[test]
    fn relinking_a_file_that_came_back_only_clears_its_status() {
        let s = setup();
        let [id] = s.import(&[("opus.ogg", "a.ogg")])[..] else {
            unreachable!()
        };
        let asset = s.asset(id);
        let hidden = s.path("hidden/a.ogg");
        fs::rename(s.path("a.ogg"), &hidden).unwrap();
        s.run(start_check(&s.jobs));
        assert_eq!(s.status(&asset), MediaStatus::Offline);

        fs::rename(&hidden, s.path("a.ogg")).unwrap();
        let changes = s.changes();
        let (files, _) = s.run(start_one(&s.jobs, id, s.path("a.ogg"), false));
        assert_eq!(files[0].outcome, MediaFileOutcome::Relinked { media: id });
        assert_eq!(
            s.changes(),
            changes,
            "same path and content: no history entry"
        );
        assert_eq!(s.status(&asset), MediaStatus::Online);
    }

    /// The done-check of task 0240: move the media folder, relink it, and the project is whole
    /// again with one action.
    #[test]
    fn a_moved_media_folder_is_relinked_by_content_in_one_step() {
        let s = setup();
        let ids = s.import(&[
            ("h264-aac.mp4", "Videos/trip/clip.mp4"),
            ("stereo-44k.mp3", "Videos/trip/music/song.mp3"),
            ("image.png", "Videos/trip/cover.png"),
            ("opus.ogg", "Elsewhere/lost.ogg"),
        ]);
        let start = s.project();
        let changes = s.changes();

        // The folder moves; one file is renamed on the way and a file under the old name but
        // with other content (same size) sits next to it. The fourth file is gone for good.
        fs::rename(s.path("Videos/trip"), s.path("Backup/trip-2026")).unwrap();
        fs::rename(
            s.path("Backup/trip-2026/clip.mp4"),
            s.path("Backup/trip-2026/renamed.mp4"),
        )
        .unwrap();
        let mut decoy = fs::read(fixture("h264-aac.mp4")).unwrap();
        let middle = decoy.len() / 2;
        decoy[middle] ^= 0xff;
        fs::write(s.path("Backup/trip-2026/a-copy/clip.mp4"), decoy).unwrap();
        fs::remove_file(s.path("Elsewhere/lost.ogg")).unwrap();

        s.run(start_check(&s.jobs));
        for asset in &start.media {
            assert_eq!(
                s.status(asset),
                MediaStatus::Offline,
                "{}",
                asset.path.absolute
            );
        }

        let folder = s.path("Backup/trip-2026");
        let (files, done) = s.run(start_folder(&s.jobs, folder.clone()));
        assert_eq!(done.result, Ok(()));
        let expected = |relative: &str| shown(&s.path(relative));
        assert_eq!(
            files,
            [
                MediaFileResult {
                    path: expected("Backup/trip-2026/renamed.mp4"),
                    outcome: MediaFileOutcome::Relinked { media: ids[0] },
                },
                MediaFileResult {
                    path: expected("Backup/trip-2026/music/song.mp3"),
                    outcome: MediaFileOutcome::Relinked { media: ids[1] },
                },
                MediaFileResult {
                    path: expected("Backup/trip-2026/cover.png"),
                    outcome: MediaFileOutcome::Relinked { media: ids[2] },
                },
                MediaFileResult {
                    path: start.media[3].path.absolute.clone(),
                    outcome: MediaFileOutcome::NotFound { media: ids[3] },
                },
            ]
        );
        assert_eq!(s.changes(), changes + 1, "one edit");
        let project = s.project();
        for (before, after) in start.media.iter().zip(&project.media).take(3) {
            assert_eq!(after.id, before.id);
            assert_eq!(after.fingerprint.hash, before.fingerprint.hash);
            assert_eq!(after.info, before.info);
            assert!(Path::new(&after.path.absolute).is_file());
        }
        assert_eq!(project.media[3], start.media[3]);
        assert_eq!(
            *s.host.added.lock().unwrap().last().unwrap(),
            ids[..3].to_vec(),
            "previews are prefetched for the relinked items"
        );

        s.run(start_check(&s.jobs));
        let project = s.project();
        let statuses: Vec<_> = project.media.iter().map(|m| s.status(m)).collect();
        use MediaStatus::*;
        assert_eq!(statuses, [Online, Online, Online, Offline]);

        // One undo puts every old path back, and with it the old (offline) status.
        let mut doc = s.host.document.lock().unwrap();
        assert_eq!(doc.undo_label(), Some(HistoryLabel::RelinkMedia));
        assert!(doc.undo());
        assert_eq!(doc.project().media, start.media);
        assert_eq!(doc.undo_label(), Some(HistoryLabel::ImportMedia));
        let undone = doc.snapshot();
        drop(doc);
        assert!(undone.media.iter().all(|m| s.status(m) == Offline));
    }

    #[test]
    fn a_folder_relink_with_nothing_missing_or_no_folder() {
        let s = setup();
        s.import(&[("opus.ogg", "a.ogg")]);
        let changes = s.changes();
        let (files, done) = s.run(start_folder(&s.jobs, s.dir.path().to_owned()));
        assert_eq!(done.result, Ok(()));
        assert!(files.is_empty());
        let (_, done) = s.run(start_folder(&s.jobs, s.dir.path().join("missing")));
        assert!(matches!(done.result, Err(IpcError::FileNotFound(_))));
        assert_eq!(s.changes(), changes);
    }
}
