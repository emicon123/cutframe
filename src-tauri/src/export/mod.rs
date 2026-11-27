//! Export in the shell (export.md §5, §6): the session's encoder probe ([`probe`]), the checks
//! of `check_export`, and the export job that `start_export` starts.
//!
//! - **`check_export`** ([`check`]): the plan for the dialog's settings against the current
//!   snapshot, and every issue at once (path rules, folder, target exists or locked, output is a
//!   source, offline media, no encoder, low disk space). File-system queries only: it runs on a
//!   blocking worker, never on the main thread, and never touches the `Document`.
//! - **`start_export`** ([`prepare`] then [`start`]): the same check; `busy` while an export
//!   runs, `fileExists` unless `overwrite`, `invalidEdit` for bad settings or a blocking issue.
//!   Then one dedicated below-normal `export` thread runs the job: it waits for the session's
//!   probe if needed (`exportPreparing`), picks the candidates and runs
//!   [`engine::export::run::run_export`] on the snapshot taken by the command.
//! - **End** (export.md §5.1): success emits `export-report` `Done` then `job-finished` `Ok`; a
//!   failure `export-report` `Failed` then `job-finished` `Err` (`io` for `diskFull` and
//!   `writeFailed`, `internal` otherwise); a cancel only `job-finished` `cancelled`.
//! - **One at a time:** [`ExportSlot`] holds the running export's job id (`busy`, the quit
//!   guard's `exporting`).

pub mod probe;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use app_core::{MediaId, Project};
use engine::export::encoders::ProbeReport;
pub use engine::export::output::free_space;
use engine::export::output::{
    TargetState, VIDEO_EXTENSION, folder_exists, is_same_file_path, target_state,
    validate_output_path,
};
use engine::export::plan::{
    AUDIO_KBPS, ExportPlan, PlanError, VideoExportSettings, output_size, plan_video_export,
    video_kbps,
};
use engine::export::run::{
    ExportError, ExportFailure, ExportObserver, ExportRequest, ExportStage, ExportSummary,
    run_export,
};
use engine::render::GpuContext;
use engine::transport::TransportShared;

pub use self::probe::EncoderProbe;
use crate::ipc::IpcError;
use crate::ipc::events::ExportReport;
use crate::ipc::types::{
    ExportCheck, ExportIssue, ExportOutcome, ExportSettings, ExportTarget, JobId, JobStage,
    MediaStatus,
};
use crate::jobs::{JobContext, JobManager, ThreadPriority};
use crate::media_status::StatusKey;

/// Free space on the volume of a folder; [`free_space`] outside tests.
pub type FreeSpace = dyn Fn(&Path) -> Option<u64> + Send + Sync;

/// The running export, if any.
#[derive(Debug, Default)]
pub struct ExportSlot {
    running: Mutex<Option<JobId>>,
}

impl ExportSlot {
    /// The job id of the running export.
    pub fn running(&self) -> Option<JobId> {
        *self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks the running export to stop (quit).
    pub fn cancel(&self, jobs: &JobManager) {
        if let Some(id) = self.running() {
            jobs.cancel(id);
        }
    }

    fn clear(&self, id: JobId) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if *running == Some(id) {
            *running = None;
        }
    }
}

/// What [`check`] found, with what `start_export` needs to start.
#[derive(Debug, Clone)]
pub struct Checked {
    pub check: ExportCheck,
    /// The plan, if the timeline is not empty.
    plan: Option<ExportPlan>,
    /// The validated output path, if it passed the path rules.
    path: Option<PathBuf>,
    settings: VideoExportSettings,
}

/// The plan and issues of `settings` for `project` (export.md §6 `check_export`). `report` is
/// the session's probe result (`None` while it runs: then no `codecUnavailable` and no encoder).
/// Errors: `invalidEdit` for a frame rate or bitrate outside the allowed values.
pub fn check(
    project: &Project,
    settings: &ExportSettings,
    report: Option<&ProbeReport>,
    free_space: &FreeSpace,
) -> Result<Checked, IpcError> {
    let ExportTarget::Video { settings: video } = settings.target;
    let video: VideoExportSettings = video.into();
    let mut issues = Vec::new();

    let plan = match plan_video_export(project, &video) {
        Ok(plan) => Some(plan),
        Err(PlanError::EmptyTimeline) => {
            issues.push(ExportIssue::EmptyTimeline);
            None
        }
        Err(err) => return Err(IpcError::InvalidEdit(err.to_string())),
    };

    let path = match validate_output_path(&settings.path, &[VIDEO_EXTENSION]) {
        Ok(path) => Some(path),
        Err(err) => {
            tracing::debug!(%err, "export path refused");
            issues.push(ExportIssue::InvalidPath);
            None
        }
    };
    let mut free_bytes = None;
    if let Some(path) = &path {
        if let Some(asset) = project
            .media
            .iter()
            .find(|a| is_same_file_path(path, Path::new(&a.path.absolute)))
        {
            issues.push(ExportIssue::OutputIsSource { media: asset.id });
        }
        if folder_exists(path) {
            match target_state(path) {
                TargetState::Free => {}
                TargetState::Exists => issues.push(ExportIssue::FileExists),
                TargetState::Locked => issues.push(ExportIssue::FileLocked),
                TargetState::NotAFile => issues.push(ExportIssue::InvalidPath),
            }
            free_bytes = path.parent().and_then(free_space);
        } else {
            issues.push(ExportIssue::FolderMissing);
        }
    }

    issues.extend(
        offline_media(project)
            .into_iter()
            .map(|media| ExportIssue::MediaOffline { media }),
    );

    let encoder = report.and_then(|report| {
        let first = report
            .candidates(video.codec, video.encoder)
            .first()
            .map(|c| c.kind);
        if first.is_none() {
            issues.push(ExportIssue::CodecUnavailable);
        }
        first
    });

    if let (Some(plan), Some(free)) = (&plan, free_bytes)
        && u128::from(free) * 10 < u128::from(plan.estimated_bytes) * 12
    {
        issues.push(ExportIssue::LowDiskSpace);
    }

    let canvas = &project.settings;
    let (width, height) = plan.map_or_else(
        || output_size(canvas.width, canvas.height, video.resolution),
        |p| (p.width, p.height),
    );
    let frame_rate = plan.map_or(canvas.frame_rate, |p| p.frame_rate);
    let check = ExportCheck {
        width,
        height,
        frame_rate,
        duration: plan.map_or(app_core::Time::ZERO, |p| p.duration()),
        frames: plan.map_or(0, |p| p.frames),
        video_kbps: plan.map_or_else(
            || video_kbps(width, height, frame_rate, video.quality, video.codec),
            |p| p.video_kbps,
        ),
        audio_kbps: AUDIO_KBPS,
        sample_rate: canvas.sample_rate,
        estimated_bytes: plan.map_or(0, |p| p.estimated_bytes).to_string(),
        free_bytes: free_bytes.map(|b| b.to_string()),
        encoder: encoder.map(Into::into),
        can_start: !issues.iter().any(|i| i.is_blocking()),
        issues,
    };
    Ok(Checked {
        check,
        plan,
        path,
        settings: video,
    })
}

/// The media files the timeline uses that are missing, in timeline order, each once.
fn offline_media(project: &Project) -> Vec<MediaId> {
    let mut seen = HashSet::new();
    project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| c.source.media())
        .filter(|&media| seen.insert(media))
        .filter(|&media| {
            project
                .media
                .iter()
                .find(|a| a.id == media)
                .is_some_and(|a| !Path::new(&a.path.absolute).is_file())
        })
        .collect()
}

/// An export that passed `start_export`'s checks.
#[derive(Debug, Clone)]
pub struct Prepared {
    project: Arc<Project>,
    plan: ExportPlan,
    path: PathBuf,
    settings: VideoExportSettings,
    overwrite: bool,
}

/// `start_export`'s checks on the snapshot `project`: `invalidEdit` for bad settings or a
/// blocking issue, `fileExists` when the target exists and `overwrite` is false.
pub fn prepare(
    project: Arc<Project>,
    settings: &ExportSettings,
    overwrite: bool,
    report: Option<&ProbeReport>,
    free_space: &FreeSpace,
) -> Result<Prepared, IpcError> {
    let checked = check(&project, settings, report, free_space)?;
    let issues = &checked.check.issues;
    if !checked.check.can_start {
        let blocking: Vec<_> = issues.iter().filter(|i| i.is_blocking()).collect();
        return Err(IpcError::InvalidEdit(format!(
            "the export cannot start: {blocking:?}"
        )));
    }
    if issues.contains(&ExportIssue::FileExists) && !overwrite {
        return Err(IpcError::FileExists(format!("{} exists", settings.path)));
    }
    let (Some(plan), Some(path)) = (checked.plan, checked.path) else {
        return Err(IpcError::Internal(
            "a startable check without a plan".into(),
        ));
    };
    Ok(Prepared {
        project,
        plan,
        path,
        settings: checked.settings,
        overwrite,
    })
}

/// What the export job uses from the app.
#[derive(Clone)]
pub struct ExportEnv {
    pub gpu: Arc<GpuContext>,
    pub probe: Arc<EncoderProbe>,
    pub slot: Arc<ExportSlot>,
    /// The preview transport (export throttles while it plays); `None` without a preview.
    pub transport: Option<Arc<TransportShared>>,
}

impl std::fmt::Debug for ExportEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportEnv").finish_non_exhaustive()
    }
}

/// Starts the export job of `prepared` and returns its id at once; `busy` while one runs.
pub fn start(jobs: &JobManager, env: ExportEnv, prepared: Prepared) -> Result<JobId, IpcError> {
    // Held while the job is registered, so the job cannot clear the slot before it is set.
    let slot = Arc::clone(&env.slot);
    let mut running = slot.running.lock().unwrap_or_else(PoisonError::into_inner);
    if running.is_some() {
        return Err(IpcError::Busy("an export is running".into()));
    }
    env.probe.report_or_start(jobs);
    let id = jobs.start_dedicated(
        JobStage::ExportPreparing,
        "export",
        ThreadPriority::BelowNormal,
        move |ctx| run_job(&ctx, &env, prepared),
    );
    *running = Some(id);
    drop(running);
    tracing::info!(job = %id.0, "export started");
    Ok(id)
}

/// The body of the export job.
fn run_job(ctx: &JobContext, env: &ExportEnv, prepared: Prepared) {
    let result = env
        .probe
        .wait(|| ctx.is_cancelled())
        .ok_or(ExportError::Cancelled)
        .and_then(|report| {
            let explicit = prepared.settings.encoder;
            let request = ExportRequest {
                project: Arc::clone(&prepared.project),
                plan: prepared.plan,
                candidates: report.candidates(prepared.settings.codec, explicit),
                explicit_encoder: explicit.is_some(),
                output: prepared.path.clone(),
                overwrite: prepared.overwrite,
            };
            tracing::info!(
                path = %request.output.display(),
                plan = ?request.plan,
                candidates = ?request.candidates,
                "export"
            );
            let observer = JobObserver {
                ctx,
                transport: env.transport.clone(),
                frames: AtomicU32::new(0),
            };
            run_export(Arc::clone(&env.gpu), request, &observer)
        });
    let id = ctx.id();
    let finished = match result {
        Ok(summary) => {
            ctx.host().export_report(ExportReport {
                job_id: id,
                outcome: done(&summary),
            });
            Ok(())
        }
        Err(ExportError::Cancelled) => Err(IpcError::Cancelled("export cancelled".into())),
        Err(ExportError::Failed(failure)) => {
            tracing::warn!(?failure, "export failed");
            if let Some(asset) = failure
                .media
                .filter(|_| failure.kind == engine::export::run::ExportErrorKind::MediaOffline)
                .and_then(|media| prepared.project.media.iter().find(|a| a.id == media))
            {
                ctx.host()
                    .media_status(vec![(StatusKey::of(asset), MediaStatus::Offline)]);
            }
            let error = job_error(&failure);
            ctx.host().export_report(ExportReport {
                job_id: id,
                outcome: failed(failure),
            });
            Err(error)
        }
    };
    env.slot.clear(id);
    ctx.clone().finish(finished);
}

fn done(summary: &ExportSummary) -> ExportOutcome {
    ExportOutcome::Done {
        path: summary.path.to_string_lossy().into_owned(),
        bytes: summary.bytes.to_string(),
        encoder: summary.encoder.into(),
        elapsed_ms: u32::try_from(summary.elapsed.as_millis()).unwrap_or(u32::MAX),
    }
}

fn failed(failure: ExportFailure) -> ExportOutcome {
    ExportOutcome::Failed {
        error: failure.kind.into(),
        media: failure.media,
        kept_path: failure.kept_path.map(|p| p.to_string_lossy().into_owned()),
        detail: failure.detail,
    }
}

/// The `job-finished` error of a failed export: `io` for disk and write errors.
fn job_error(failure: &ExportFailure) -> IpcError {
    use engine::export::run::ExportErrorKind as Kind;
    match failure.kind {
        Kind::DiskFull | Kind::WriteFailed => IpcError::Io(failure.detail.clone()),
        _ => IpcError::Internal(failure.detail.clone()),
    }
}

/// The export loop's observer over the job context.
struct JobObserver<'a> {
    ctx: &'a JobContext,
    transport: Option<Arc<TransportShared>>,
    frames: AtomicU32,
}

impl ExportObserver for JobObserver<'_> {
    fn cancelled(&self) -> bool {
        self.ctx.is_cancelled()
    }

    fn stage(&self, stage: ExportStage) {
        let stage = match stage {
            ExportStage::Preparing => JobStage::ExportPreparing,
            ExportStage::Encoding => JobStage::ExportEncoding,
            ExportStage::Finalizing => JobStage::ExportFinalizing,
        };
        self.ctx.set_stage(stage);
        // Progress only while encoding (export.md §5.1).
        if stage != JobStage::ExportEncoding {
            self.ctx.set_total(0);
        } else {
            self.ctx
                .set_total(self.frames.load(Ordering::Relaxed) as usize);
        }
    }

    fn set_total(&self, frames: u32) {
        self.frames.store(frames, Ordering::Relaxed);
        self.ctx.set_total(frames as usize);
    }

    fn frame_done(&self) {
        self.ctx.step_done();
    }

    fn playback_active(&self) -> bool {
        self.transport.as_ref().is_some_and(|t| t.is_playing())
    }
}

#[cfg(test)]
mod tests;
