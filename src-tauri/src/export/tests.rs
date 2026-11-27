use std::sync::OnceLock;
use std::time::Duration;

use app_core::media::{MediaPath, samples};
use app_core::{Clip, ClipSource, FLICKS_PER_SECOND, FrameRate, Rgba8, Time};
use engine::export::encoders::{EncoderError, EncoderFilter, EncoderKind, OptionSet};
use engine::render::AdapterMode;

use super::*;
use crate::ipc::events::JobFinished;
use crate::ipc::types::{EncoderKind as EncoderView, ExportQuality, ExportResolution, VideoCodec};
use crate::jobs::tests::{FakeHost, Recorded, wait_finished};

fn warp() -> Arc<GpuContext> {
    static GPU: OnceLock<Arc<GpuContext>> = OnceLock::new();
    Arc::clone(
        GPU.get_or_init(|| {
            Arc::new(GpuContext::new_with(AdapterMode::SoftwareOnly).expect("WARP"))
        }),
    )
}

fn seconds(s: i64) -> Time {
    Time(s * FLICKS_PER_SECOND)
}

/// 160 x 90, 30 fps, `length` seconds of colour clips.
fn project(length: i64) -> Project {
    let mut project = Project::new("Export");
    project.settings.width = 160;
    project.settings.height = 90;
    project.settings.frame_rate = FrameRate::FPS_30;
    project.tracks[0].clips = vec![Clip::color(
        "Colour",
        Time::ZERO,
        seconds(length),
        Rgba8::rgb(0x40, 0x80, 0xc0),
    )];
    project
}

fn settings(path: &Path) -> ExportSettings {
    ExportSettings {
        path: path.to_string_lossy().into_owned(),
        target: ExportTarget::Video {
            settings: crate::ipc::types::VideoExportSettings {
                codec: VideoCodec::H264,
                resolution: ExportResolution::Project,
                frame_rate: None,
                quality: ExportQuality::Low,
                encoder: None,
            },
        },
    }
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX / 2)
}

fn report(available: &[EncoderKind]) -> ProbeReport {
    ProbeReport::run_with(&EncoderFilter::default(), |kind, _| {
        if available.contains(&kind) {
            Ok(OptionSet::Full)
        } else {
            Err(EncoderError::NotInBuild("fake"))
        }
    })
}

#[test]
fn a_clean_check_has_the_plan_the_first_encoder_and_no_issues() {
    let dir = tempfile::tempdir().unwrap();
    let report = report(&[EncoderKind::Qsv, EncoderKind::OpenH264]);
    let checked = check(
        &project(2),
        &settings(&dir.path().join("Vlog")),
        Some(&report),
        &plenty,
    )
    .unwrap();
    let check = checked.check;
    assert_eq!((check.width, check.height, check.frames), (160, 90, 60));
    assert_eq!(check.duration, seconds(2));
    assert_eq!(check.audio_kbps, 384);
    assert_eq!(check.encoder, Some(EncoderView::Qsv));
    assert!(check.issues.is_empty(), "{:?}", check.issues);
    assert!(check.can_start);
    assert!(check.free_bytes.is_some());
    assert!(check.estimated_bytes.parse::<u64>().unwrap() > 0);
    assert_eq!(
        checked.path.unwrap(),
        dir.path().join("Vlog.mp4"),
        ".mp4 appended"
    );

    // Before the probe finished: no encoder and no codecUnavailable.
    let pending = check_of(&project(2), &dir.path().join("a.mp4"), None, &plenty);
    assert_eq!((pending.encoder, pending.can_start), (None, true));
}

fn check_of(
    project: &Project,
    path: &Path,
    report: Option<&ProbeReport>,
    free: &FreeSpace,
) -> ExportCheck {
    check(project, &settings(path), report, free).unwrap().check
}

#[test]
fn the_check_lists_every_issue() {
    let dir = tempfile::tempdir().unwrap();
    let report = report(&[EncoderKind::OpenH264]);
    let ok = Some(&report);

    // Empty timeline, invalid path, missing folder.
    let empty = check_of(
        &Project::new("Empty"),
        &dir.path().join("a.mp4"),
        ok,
        &plenty,
    );
    assert_eq!(empty.issues, [ExportIssue::EmptyTimeline]);
    assert!(!empty.can_start);
    assert_eq!(empty.frames, 0);
    for bad in ["relative.mp4", "C:\\Videos\\clip.exe"] {
        let checked = check(&project(1), &settings(Path::new(bad)), ok, &plenty).unwrap();
        assert_eq!(checked.check.issues, [ExportIssue::InvalidPath], "{bad}");
    }
    let missing = check_of(
        &project(1),
        &dir.path().join("no").join("a.mp4"),
        ok,
        &plenty,
    );
    assert_eq!(missing.issues, [ExportIssue::FolderMissing]);

    // Offline media and the output is a source file.
    let mut with_media = project(1);
    let offline = samples::video(1);
    let mut source = samples::video(2);
    let source_path = dir.path().join("source.mp4");
    std::fs::write(&source_path, b"video").unwrap();
    source.path = MediaPath::absolute(source_path.to_string_lossy());
    for asset in [&offline, &source] {
        with_media.tracks[0].clips.push(Clip {
            source: ClipSource::media_at(asset.id, Time::ZERO),
            ..Clip::color("Media", seconds(1), seconds(1), Rgba8::BLACK)
        });
    }
    with_media.media = vec![offline.clone(), source.clone()];
    let found = check_of(&with_media, &dir.path().join("SOURCE.MP4"), ok, &plenty);
    assert_eq!(
        found.issues,
        [
            ExportIssue::OutputIsSource { media: source.id },
            ExportIssue::FileExists,
            ExportIssue::MediaOffline { media: offline.id },
        ]
    );
    assert!(!found.can_start);

    // An existing file is not blocking; one held open is.
    let target = dir.path().join("exists.mp4");
    std::fs::write(&target, b"old").unwrap();
    let exists = check_of(&project(1), &target, ok, &plenty);
    assert_eq!(exists.issues, [ExportIssue::FileExists]);
    assert!(exists.can_start);
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        let locked = check_of(&project(1), &target, ok, &plenty);
        assert_eq!(locked.issues, [ExportIssue::FileLocked]);
    }

    // No encoder for the codec, or the chosen one is not available.
    let none = report_none();
    let unavailable = check_of(&project(1), &dir.path().join("b.mp4"), Some(&none), &plenty);
    assert_eq!(unavailable.issues, [ExportIssue::CodecUnavailable]);
    let mut nvenc = settings(&dir.path().join("c.mp4"));
    let ExportTarget::Video { settings: video } = &mut nvenc.target;
    video.encoder = Some(EncoderView::Nvenc);
    let checked = check(&project(1), &nvenc, ok, &plenty).unwrap().check;
    assert_eq!(checked.issues, [ExportIssue::CodecUnavailable]);

    // Less than 1.2 x the estimate free: a warning only.
    let tight = |_: &Path| Some(1000);
    let low = check_of(&project(1), &dir.path().join("d.mp4"), ok, &tight);
    assert_eq!(low.issues, [ExportIssue::LowDiskSpace]);
    assert!(low.can_start);
    assert_eq!(low.free_bytes.as_deref(), Some("1000"));
}

fn report_none() -> ProbeReport {
    report(&[])
}

#[test]
fn bad_settings_are_invalid_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut odd_rate = settings(&dir.path().join("a.mp4"));
    let ExportTarget::Video { settings: video } = &mut odd_rate.target;
    video.frame_rate = Some(FrameRate::new(48, 1).unwrap());
    assert!(matches!(
        check(&project(1), &odd_rate, None, &plenty),
        Err(IpcError::InvalidEdit(_))
    ));
    let mut bitrate = settings(&dir.path().join("a.mp4"));
    let ExportTarget::Video { settings: video } = &mut bitrate.target;
    video.quality = ExportQuality::Custom { kbps: 100 };
    assert!(matches!(
        prepare(Arc::new(project(1)), &bitrate, false, None, &plenty),
        Err(IpcError::InvalidEdit(_))
    ));
}

#[test]
fn prepare_refuses_blocking_issues_and_existing_files_without_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let empty = prepare(
        Arc::new(Project::new("Empty")),
        &settings(&dir.path().join("a.mp4")),
        true,
        None,
        &plenty,
    );
    assert!(matches!(empty, Err(IpcError::InvalidEdit(_))), "{empty:?}");
    let target = dir.path().join("exists.mp4");
    std::fs::write(&target, b"old").unwrap();
    let refused = prepare(
        Arc::new(project(1)),
        &settings(&target),
        false,
        None,
        &plenty,
    );
    assert!(
        matches!(refused, Err(IpcError::FileExists(_))),
        "{refused:?}"
    );
    assert!(
        prepare(
            Arc::new(project(1)),
            &settings(&target),
            true,
            None,
            &plenty
        )
        .is_ok()
    );
}

/// The probe of the tests: OpenH264 only, after 100 ms.
fn env(jobs: &JobManager) -> ExportEnv {
    let probe = Arc::new(EncoderProbe::with_runner(probe::tests::fake_runner));
    probe.report_or_start(jobs);
    ExportEnv {
        gpu: warp(),
        probe,
        slot: Arc::default(),
        transport: None,
    }
}

#[test]
fn an_export_reports_its_stages_then_the_report_then_finishes_and_a_second_one_is_busy() {
    let dir = tempfile::tempdir().unwrap();
    let (host, rx) = FakeHost::new();
    let jobs = JobManager::with_workers(host.clone(), 1);
    let env = env(&jobs);
    let target = dir.path().join("out.mp4");
    let prepared = prepare(
        Arc::new(project(2)),
        &settings(&target),
        false,
        None,
        &plenty,
    )
    .unwrap();
    let id = start(&jobs, env.clone(), prepared.clone()).unwrap();
    assert_eq!(env.slot.running(), Some(id));
    assert!(matches!(
        start(&jobs, env.clone(), prepared),
        Err(IpcError::Busy(_))
    ));
    // The probe's job finishes first, then the export's.
    let mut finished = wait_finished(&rx);
    if finished != id {
        finished = wait_finished(&rx);
    }
    assert_eq!(finished, id);
    assert_eq!(env.slot.running(), None, "the slot is free again");

    let events: Vec<Recorded> = host
        .events()
        .into_iter()
        .filter(|e| match e {
            Recorded::Progress(p) => p.job_id == id,
            Recorded::Finished(f) => f.job_id == id,
            Recorded::Export(_) => true,
            _ => false,
        })
        .collect();
    let mut stages: Vec<JobStage> = events
        .iter()
        .filter_map(|e| match e {
            Recorded::Progress(p) => Some(p.stage),
            _ => None,
        })
        .collect();
    stages.dedup();
    let order: Vec<JobStage> = [
        JobStage::ExportPreparing,
        JobStage::ExportEncoding,
        JobStage::ExportFinalizing,
    ]
    .into_iter()
    .filter(|s| stages.contains(s))
    .collect();
    assert_eq!(stages, order, "stages in order: {stages:?}");
    assert!(events.iter().all(|e| match e {
        Recorded::Progress(p) => (p.stage == JobStage::ExportEncoding) == p.progress.is_some(),
        _ => true,
    }));
    let n = events.len();
    match (&events[n - 2], &events[n - 1]) {
        (
            Recorded::Export(ExportReport {
                job_id,
                outcome:
                    ExportOutcome::Done {
                        encoder,
                        bytes,
                        path,
                        ..
                    },
            }),
            Recorded::Finished(JobFinished { result: Ok(()), .. }),
        ) => {
            assert_eq!(*job_id, id);
            assert_eq!(*encoder, EncoderView::OpenH264);
            assert!(bytes.parse::<u64>().unwrap() > 0);
            assert_eq!(Path::new(path), target);
        }
        other => panic!("report then finished, got {other:?}"),
    }
    assert!(target.is_file());
}

#[test]
fn a_failed_export_reports_the_error_and_finishes_with_io_or_internal() {
    let dir = tempfile::tempdir().unwrap();
    let (host, rx) = FakeHost::new();
    let jobs = JobManager::with_workers(host.clone(), 1);
    let env = env(&jobs);
    // The media goes offline after the check.
    let copy = dir.path().join("clip.mp4");
    std::fs::write(&copy, b"video").unwrap();
    let mut asset = samples::video(3);
    asset.path = MediaPath::absolute(copy.to_string_lossy());
    let mut project = project(1);
    project.tracks[0].clips.push(Clip {
        source: ClipSource::media_at(asset.id, Time::ZERO),
        ..Clip::color("Media", seconds(1), seconds(1), Rgba8::BLACK)
    });
    project.media = vec![asset.clone()];
    let prepared = prepare(
        Arc::new(project),
        &settings(&dir.path().join("out.mp4")),
        false,
        None,
        &plenty,
    )
    .unwrap();
    std::fs::remove_file(&copy).unwrap();
    let id = start(&jobs, env, prepared).unwrap();
    while wait_finished(&rx) != id {}
    let events = host.events();
    let report = events.iter().find_map(|e| match e {
        Recorded::Export(r) => Some(r.clone()),
        _ => None,
    });
    match report.map(|r| r.outcome) {
        Some(ExportOutcome::Failed { error, media, .. }) => {
            assert_eq!(error, crate::ipc::types::ExportErrorKind::MediaOffline);
            assert_eq!(media, Some(asset.id));
        }
        other => panic!("{other:?}"),
    }
    assert!(events.iter().any(|e| matches!(e,
        Recorded::Finished(JobFinished { job_id, result: Err(IpcError::Internal(_)) }) if *job_id == id)));
    assert_eq!(
        host.statuses.lock().unwrap().get(&asset),
        MediaStatus::Offline,
        "the media panel shows it offline"
    );
}

#[test]
fn a_cancelled_export_has_no_report_and_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let (host, rx) = FakeHost::new();
    let jobs = JobManager::with_workers(host.clone(), 1);
    let env = env(&jobs);
    let prepared = prepare(
        Arc::new(project(30)),
        &settings(&dir.path().join("long.mp4")),
        false,
        None,
        &plenty,
    )
    .unwrap();
    let id = start(&jobs, env.clone(), prepared).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    env.slot.cancel(&jobs);
    while wait_finished(&rx) != id {}
    let events = host.events();
    assert!(!events.iter().any(|e| matches!(e, Recorded::Export(_))));
    assert!(events.contains(&Recorded::Finished(JobFinished {
        job_id: id,
        result: Err(IpcError::Cancelled("export cancelled".into())),
    })));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no partial file"
    );
}
