//! The import job (design §6.2): expand folders, fingerprint and probe every file on the pool,
//! then add all successes as one undoable `AddMedia` (history entry `importMedia`).
//!
//! - Files named explicitly are always tried and reported if they fail; inside folders only files
//!   with a supported extension are taken, silently.
//! - Content already in the project, or earlier in the same import, is skipped as
//!   `alreadyImported` (by fingerprint hash).
//! - Cancel adds nothing. While a gesture is open the add waits for it to end (ADR-0006 §3).
//! - `media-job-report` (per-file outcomes in input order) comes just before `job-finished`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::MediaId;
use app_core::command::AddMedia;
use app_core::media::{Fingerprint, MediaAsset, MediaPath, validate_asset};
use engine::media::{MediaError, Probed, fingerprint, has_media_extension, probe_cancellable};

use crate::ipc::IpcError;
use crate::ipc::events::MediaJobReport;
use crate::ipc::types::{JobId, JobStage, MediaErrorKind, MediaFileOutcome, MediaFileResult};
use crate::jobs::{JobContext, JobManager, Priority};

/// Most media files one dropped folder may hold (recursively). A larger folder is reported as one
/// `tooManyFiles` failure and nothing from it is imported.
pub const MAX_IMPORT_FILES: usize = 2000;

/// How often the finish step retries while a gesture is open.
const GESTURE_RETRY: Duration = Duration::from_millis(50);

/// Starts importing `paths` (files or folders). Returns at once.
pub fn start(jobs: &JobManager, paths: Vec<PathBuf>) -> JobId {
    jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
        plan(ctx, &paths);
    })
}

/// One entry of the import, in input order.
#[derive(Debug)]
enum Planned {
    File { path: PathBuf, shown: String },
    Failed(MediaFileResult),
}

/// What happened to one file on the pool.
#[derive(Debug)]
enum FileResult {
    Probed {
        fingerprint: Fingerprint,
        probed: Box<Probed>,
    },
    Failed(MediaError),
    Cancelled,
}

/// Paths as stored and shown: absolute, without the Windows `\\?\` prefix.
fn normalise(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let path = match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_owned(),
    };
    std::path::absolute(&path).unwrap_or(path)
}

/// The key that makes two spellings of one path the same file (Windows paths ignore case).
fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

fn shown(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn failed(path: &Path, error: MediaErrorKind, detail: impl Into<String>) -> Planned {
    Planned::Failed(MediaFileResult {
        path: shown(path),
        outcome: MediaFileOutcome::Failed {
            error,
            detail: detail.into(),
        },
    })
}

/// Media files under `dir`, recursively, sorted. Symlinked folders are not followed (no loops).
/// `Err` if there are more than `limit`.
fn walk(dir: &Path, limit: usize) -> std::io::Result<Result<Vec<PathBuf>, ()>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && has_media_extension(&path) {
                files.push(path);
                if files.len() > limit {
                    return Ok(Err(()));
                }
            }
        }
    }
    files.sort();
    Ok(Ok(files))
}

/// Expands folders and drops repeated paths, keeping the input order.
fn expand(paths: &[PathBuf]) -> Vec<Planned> {
    let mut planned = Vec::new();
    let mut seen = HashSet::new();
    let mut add_file = |planned: &mut Vec<Planned>, path: PathBuf| {
        if seen.insert(path_key(&path)) {
            let shown = shown(&path);
            if path.to_str().is_some() {
                planned.push(Planned::File { path, shown });
            } else {
                planned.push(failed(
                    &path,
                    MediaErrorKind::UnsupportedPath,
                    "not valid UTF-8",
                ));
            }
        }
    };
    for path in paths.iter().map(|p| normalise(p)) {
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => match walk(&path, MAX_IMPORT_FILES) {
                Ok(Ok(files)) => {
                    for file in files {
                        add_file(&mut planned, file);
                    }
                }
                Ok(Err(())) => planned.push(failed(
                    &path,
                    MediaErrorKind::TooManyFiles,
                    format!("more than {MAX_IMPORT_FILES} media files"),
                )),
                Err(err) => planned.push(Planned::Failed(MediaFileResult {
                    path: shown(&path),
                    outcome: io_failure(&err),
                })),
            },
            // Files, and paths whose stat failed: the per-file task reports the error.
            _ => add_file(&mut planned, path),
        }
    }
    planned
}

fn io_failure(err: &std::io::Error) -> MediaFileOutcome {
    let err = MediaError::from_io(err);
    MediaFileOutcome::Failed {
        error: err.kind.into(),
        detail: err.detail,
    }
}

fn probe_file(path: &Path, ctx: &JobContext) -> FileResult {
    if ctx.is_cancelled() {
        return FileResult::Cancelled;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let fingerprint = fingerprint(path)?;
        let probed = probe_cancellable(path, ctx.cancel_flag())?;
        Ok::<_, MediaError>((fingerprint, probed))
    }));
    match result {
        Ok(Ok((fingerprint, probed))) => FileResult::Probed {
            fingerprint,
            probed: Box::new(probed),
        },
        Ok(Err(err)) if err.kind == engine::media::MediaErrorKind::Cancelled => {
            FileResult::Cancelled
        }
        Ok(Err(err)) => FileResult::Failed(err),
        Err(_) => FileResult::Failed(MediaError::new(
            engine::media::MediaErrorKind::Damaged,
            "internal error while probing",
        )),
    }
}

/// First task: expand, then queue one probe task per file. The last task to finish runs
/// [`finish`].
fn plan(ctx: JobContext, paths: &[PathBuf]) {
    let planned = Arc::new(expand(paths));
    let files: Vec<usize> = planned
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Planned::File { .. }).then_some(i))
        .collect();
    ctx.set_total(files.len());
    let results: Arc<Mutex<HashMap<usize, FileResult>>> = Arc::default();
    if files.is_empty() {
        finish(ctx, &planned, HashMap::new());
        return;
    }
    let remaining = Arc::new(std::sync::atomic::AtomicUsize::new(files.len()));
    for index in files {
        let (planned, results, remaining) = (
            Arc::clone(&planned),
            Arc::clone(&results),
            Arc::clone(&remaining),
        );
        ctx.spawn(Priority::Interactive, move |ctx| {
            let Planned::File { path, .. } = &planned[index] else {
                unreachable!("only files are queued");
            };
            let result = probe_file(path, &ctx);
            results
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(index, result);
            ctx.step_done();
            if remaining.fetch_sub(1, std::sync::atomic::Ordering::AcqRel) == 1 {
                let results =
                    std::mem::take(&mut *results.lock().unwrap_or_else(|p| p.into_inner()));
                finish(ctx, &planned, results);
            }
        });
    }
}

/// One probed file ready to become an asset.
#[derive(Debug, Clone)]
struct Candidate {
    index: usize,
    path: String,
    fingerprint: Fingerprint,
    probed: Probed,
}

/// Decides each candidate against the project's media: new content becomes an asset with a new id
/// (in input order), content already present or earlier in the batch is `alreadyImported`.
fn plan_assets(
    existing: &[MediaAsset],
    candidates: &[Candidate],
) -> (Vec<MediaAsset>, Vec<(usize, MediaFileOutcome)>) {
    let mut by_hash: HashMap<&str, MediaId> = existing
        .iter()
        .map(|m| (m.fingerprint.hash.as_str(), m.id))
        .collect();
    let mut assets = Vec::new();
    let mut outcomes = Vec::new();
    for candidate in candidates {
        if let Some(&media) = by_hash.get(candidate.fingerprint.hash.as_str()) {
            outcomes.push((candidate.index, MediaFileOutcome::AlreadyImported { media }));
            continue;
        }
        let asset = MediaAsset {
            id: MediaId::new(),
            kind: candidate.probed.kind,
            path: MediaPath::absolute(candidate.path.clone()),
            fingerprint: candidate.fingerprint.clone(),
            info: candidate.probed.info.clone(),
        };
        if let Err(problem) = validate_asset(&asset) {
            // A probe result the model rejects is a bug in the probe; never add it.
            tracing::error!(path = %candidate.path, %problem, "probe produced an invalid asset");
            outcomes.push((
                candidate.index,
                MediaFileOutcome::Failed {
                    error: MediaErrorKind::Damaged,
                    detail: format!("invalid probe result: {problem}"),
                },
            ));
            continue;
        }
        by_hash.insert(&candidate.fingerprint.hash, asset.id);
        outcomes.push((candidate.index, MediaFileOutcome::Added { media: asset.id }));
        assets.push(asset);
    }
    (assets, outcomes)
}

fn finish(ctx: JobContext, planned: &[Planned], mut results: HashMap<usize, FileResult>) {
    if ctx.is_cancelled() {
        ctx.finish(Err(IpcError::Cancelled("import cancelled".into())));
        return;
    }
    let mut candidates = Vec::new();
    let mut outcomes: Vec<Option<MediaFileOutcome>> = vec![None; planned.len()];
    for (index, entry) in planned.iter().enumerate() {
        match entry {
            Planned::Failed(result) => outcomes[index] = Some(result.outcome.clone()),
            Planned::File { shown, .. } => match results.remove(&index) {
                Some(FileResult::Probed {
                    fingerprint,
                    probed,
                }) => candidates.push(Candidate {
                    index,
                    path: shown.clone(),
                    fingerprint,
                    probed: *probed,
                }),
                Some(FileResult::Failed(err)) => {
                    outcomes[index] = Some(MediaFileOutcome::Failed {
                        error: err.kind.into(),
                        detail: err.detail,
                    });
                }
                // A file cancelled by FFmpeg's interrupt while the flag was set: the job is
                // cancelled (checked above), so this cannot be reached. Report it as damaged
                // rather than panic.
                Some(FileResult::Cancelled) | None => {
                    outcomes[index] = Some(MediaFileOutcome::Failed {
                        error: MediaErrorKind::Damaged,
                        detail: "probe did not finish".into(),
                    });
                }
            },
        }
    }

    // Decide and add under the document lock, so two concurrent imports of the same file cannot
    // both add it. Retry while a gesture is open.
    let decided = loop {
        let mut decided = None;
        let edit = ctx.host().edit(&mut |doc| {
            if doc.gesture_label().is_some() {
                return Ok(false);
            }
            let (assets, decisions) = plan_assets(&doc.project().media, &candidates);
            let changed = !assets.is_empty();
            if changed {
                doc.execute(Box::new(AddMedia::new(assets)))?;
            }
            decided = Some(decisions);
            Ok(changed)
        });
        if let Err(err) = edit {
            tracing::error!(%err, "adding imported media failed");
            ctx.finish(Err(err));
            return;
        }
        if let Some(decided) = decided {
            break decided;
        }
        if ctx.is_cancelled() {
            ctx.finish(Err(IpcError::Cancelled("import cancelled".into())));
            return;
        }
        std::thread::sleep(GESTURE_RETRY);
    };
    for (index, outcome) in decided {
        outcomes[index] = Some(outcome);
    }

    let files = planned
        .iter()
        .zip(outcomes)
        .map(|(entry, outcome)| MediaFileResult {
            path: match entry {
                Planned::File { shown, .. } => shown.clone(),
                Planned::Failed(result) => result.path.clone(),
            },
            outcome: outcome.expect("every entry has an outcome"),
        })
        .collect();
    ctx.host().media_job_report(MediaJobReport {
        job_id: ctx.id(),
        files,
    });
    ctx.finish(Ok(()));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use app_core::command::RenameProject;

    use super::*;
    use crate::ipc::events::JobFinished;
    use crate::jobs::tests::{FakeHost, Recorded, wait_finished};

    fn media_ids(project: &app_core::Project) -> Vec<MediaId> {
        project.media.iter().map(|m| m.id).collect()
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/engine/tests/fixtures/media")
            .join(name)
    }

    fn report(host: &FakeHost, id: JobId) -> Vec<MediaFileResult> {
        let events = host.events();
        let position = |pred: &dyn Fn(&Recorded) -> bool| events.iter().position(pred);
        let report = position(&|e| matches!(e, Recorded::Report(r) if r.job_id == id))
            .expect("media-job-report");
        let finished = position(&|e| matches!(e, Recorded::Finished(f) if f.job_id == id))
            .expect("job-finished");
        assert!(report < finished, "the report comes before job-finished");
        match &events[report] {
            Recorded::Report(r) => r.files.clone(),
            _ => unreachable!(),
        }
    }

    fn finished(host: &FakeHost, id: JobId) -> JobFinished {
        host.events()
            .into_iter()
            .find_map(|e| match e {
                Recorded::Finished(f) if f.job_id == id => Some(f),
                _ => None,
            })
            .expect("job-finished")
    }

    fn kinds(files: &[MediaFileResult]) -> Vec<String> {
        files
            .iter()
            .map(|f| match &f.outcome {
                MediaFileOutcome::Added { .. } => "added".to_owned(),
                MediaFileOutcome::AlreadyImported { .. } => "alreadyImported".to_owned(),
                MediaFileOutcome::Failed { error, .. } => format!("failed:{error:?}"),
            })
            .collect()
    }

    #[test]
    fn a_mixed_import_is_one_history_entry_with_outcomes_in_input_order() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 3);
        let paths = vec![
            fixture("h264-aac.mp4"),
            fixture("text.mov"),
            fixture("stereo-44k.mp3"),
            fixture("h264-aac.mp4"), // same path twice: dropped silently
            fixture("exif-orientation-6.jpg"),
            fixture("does-not-exist.mov"),
            fixture("ąęłó/zażółć.mp4"),
        ];
        let id = start(&jobs, paths);
        assert_eq!(wait_finished(&rx), id);
        assert_eq!(finished(&host, id).result, Ok(()));

        let files = report(&host, id);
        assert_eq!(
            kinds(&files),
            [
                "added",
                "failed:UnsupportedFormat",
                "added",
                "added",
                "failed:FileNotFound",
                "added"
            ]
        );
        assert!(files[0].path.ends_with("h264-aac.mp4"));
        assert!(Path::new(&files[0].path).is_absolute());

        let doc = host.document.lock().unwrap();
        let added: Vec<MediaId> = files
            .iter()
            .filter_map(|f| match f.outcome {
                MediaFileOutcome::Added { media } => Some(media),
                _ => None,
            })
            .collect();
        assert_eq!(media_ids(doc.project()), added, "in input order");
        assert_eq!(
            doc.undo_label(),
            Some(app_core::command::HistoryLabel::ImportMedia)
        );
        assert_eq!(host.changes.load(Ordering::Relaxed), 1, "one edit");
        let kinds: Vec<_> = doc.project().media.iter().map(|m| m.kind).collect();
        use app_core::MediaKind::*;
        assert_eq!(kinds, [Video, Audio, Image, Video]);
        drop(doc);

        // One undo removes the whole import.
        let mut doc = host.document.lock().unwrap();
        assert!(doc.undo());
        assert!(doc.project().media.is_empty());
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(media_ids(doc.project()), added, "redo keeps the ids");
    }

    #[test]
    fn duplicates_by_content_are_skipped_in_the_batch_and_against_the_project() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 2);
        let first = start(&jobs, vec![fixture("aac.m4a")]);
        wait_finished(&rx);
        let existing = match report(&host, first)[0].outcome {
            MediaFileOutcome::Added { media } => media,
            ref other => panic!("{other:?}"),
        };

        // Same content under another name (a copy) in the same batch, and the file again.
        let dir = std::env::temp_dir().join(format!("cutframe-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy_a = dir.join("copy-a.mp3");
        let copy_b = dir.join("copy-b.mp3");
        std::fs::copy(fixture("stereo-44k.mp3"), &copy_a).unwrap();
        std::fs::copy(fixture("stereo-44k.mp3"), &copy_b).unwrap();
        let second = start(&jobs, vec![fixture("aac.m4a"), copy_a, copy_b]);
        wait_finished(&rx);
        let files = report(&host, second);
        assert_eq!(
            kinds(&files),
            ["alreadyImported", "added", "alreadyImported"]
        );
        assert_eq!(
            files[0].outcome,
            MediaFileOutcome::AlreadyImported { media: existing }
        );
        let MediaFileOutcome::Added { media: copy } = files[1].outcome else {
            unreachable!()
        };
        assert_eq!(
            files[2].outcome,
            MediaFileOutcome::AlreadyImported { media: copy }
        );
        assert_eq!(host.document.lock().unwrap().project().media.len(), 2);

        // Everything already there: no edit, no history entry.
        let changes = host.changes.load(Ordering::Relaxed);
        let third = start(&jobs, vec![fixture("aac.m4a")]);
        wait_finished(&rx);
        assert_eq!(kinds(&report(&host, third)), ["alreadyImported"]);
        assert_eq!(host.changes.load(Ordering::Relaxed), changes);
    }

    #[test]
    fn folders_expand_to_their_media_files_only() {
        let dir = std::env::temp_dir().join(format!("cutframe-folder-{}", std::process::id()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::copy(fixture("image.png"), dir.join("b.png")).unwrap();
        std::fs::copy(fixture("opus.ogg"), nested.join("a.ogg")).unwrap();
        std::fs::write(dir.join("notes.txt"), "not media").unwrap();

        let planned = expand(std::slice::from_ref(&dir));
        let names: Vec<_> = planned
            .iter()
            .map(|p| match p {
                Planned::File { path, .. } => {
                    path.file_name().unwrap().to_string_lossy().into_owned()
                }
                Planned::Failed(f) => format!("failed {}", f.path),
            })
            .collect();
        // Sorted by path: `dir/b.png` < `dir/nested/a.ogg`; the text file is skipped.
        assert_eq!(names, ["b.png", "a.ogg"]);
        assert!(!names.iter().any(|n| n.contains("notes")));

        assert!(matches!(walk(&dir, 1).unwrap(), Err(())), "over the limit");
    }

    #[test]
    fn cancel_adds_nothing_and_reports_cancelled() {
        let (host, rx) = FakeHost::new();
        // No worker is free: the job is still queued when it is cancelled.
        let jobs = JobManager::with_workers(host.clone(), 1);
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let gate_rx = Mutex::new(gate_rx);
        let blocker = jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
            gate_rx.lock().unwrap().recv().ok();
            ctx.finish(Ok(()));
        });
        let id = start(&jobs, vec![fixture("h264-aac.mp4"), fixture("opus.ogg")]);
        jobs.cancel(id);
        gate_tx.send(()).unwrap();
        let mut done = vec![wait_finished(&rx), wait_finished(&rx)];
        done.sort_by_key(|id| id.0);
        let mut expected = vec![blocker, id];
        expected.sort_by_key(|id| id.0);
        assert_eq!(done, expected);

        assert_eq!(
            finished(&host, id).result,
            Err(IpcError::Cancelled("import cancelled".into()))
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|e| matches!(e, Recorded::Report(_)))
        );
        assert!(host.document.lock().unwrap().project().media.is_empty());
        assert_eq!(host.changes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_add_waits_for_an_open_gesture() {
        let (host, rx) = FakeHost::new();
        host.document
            .lock()
            .unwrap()
            .gesture_update(Box::new(RenameProject::new("Dragging")))
            .unwrap();
        let jobs = JobManager::with_workers(host.clone(), 1);
        let id = start(&jobs, vec![fixture("mono-48k.wav")]);

        std::thread::sleep(Duration::from_millis(300));
        assert!(
            host.document.lock().unwrap().project().media.is_empty(),
            "deferred"
        );
        assert!(rx.try_recv().is_err(), "still running");

        host.document.lock().unwrap().gesture_cancel();
        assert_eq!(wait_finished(&rx), id);
        let doc = host.document.lock().unwrap();
        assert_eq!(doc.project().media.len(), 1);
        assert_eq!(doc.project().name, "Test");
    }

    #[test]
    fn windows_long_path_prefix_is_removed() {
        let path = normalise(Path::new(r"\\?\C:\Videos\a.mp4"));
        let text = path.to_string_lossy();
        assert!(!text.contains(r"\\?\"), "{text}");
        assert!(text.ends_with(r"C:\Videos\a.mp4"), "{text}");
        #[cfg(windows)]
        assert_eq!(text, r"C:\Videos\a.mp4");
    }
}
