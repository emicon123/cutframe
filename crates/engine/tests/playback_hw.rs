//! Playback on real hardware (playback.md §11): the render thread's playback step with a fake
//! 60 Hz vsync and a wall clock, driving the real `ReaderPool` (hardware decode) and
//! `Compositor` offscreen at a 1280 x 720 preview size, plus the decode-ahead scheduler. Counts
//! dropped frames as the preview does (skipped at 1x, late = composited with a stale or missing
//! layer). Ignored by default: it needs a GPU with video decode and clips outside the repository.
//!
//! ```text
//! CUTFRAME_GPU=integrated CUTFRAME_PLAYBACK=hlg30 \
//!   CUTFRAME_PLAYBACK_MAIN=C:\clips\hevc-4k30-hlg-60s.mp4 \
//!   CUTFRAME_PLAYBACK_OVERLAY=C:\clips\h264-1080p30.mp4 \
//!   cargo test --release -p engine --test playback_hw -- --ignored --nocapture
//! ```
//!
//! Scenarios (`CUTFRAME_PLAYBACK`): `main` (the main clip, plus the overlay clip on a track above
//! it when given; project rate `CUTFRAME_PLAYBACK_FPS`, default 30), `jumpcuts` (2 s clips of
//! the main clip at scattered source times). `CUTFRAME_PLAYBACK_SECONDS` limits the run (in
//! source seconds of the main clip). In `main`, `CUTFRAME_PLAYBACK_SPEED` (percent, default 100)
//! and `CUTFRAME_PLAYBACK_REVERSE=1` set the main clip's speed and direction
//! (`clip-speed-audio.md` §8: 2x and 0.5x, reverse 1080p and 4K through the GOP-reverse reader).
//! The run prints the process's working set before and its peak after.
//!
//! `CUTFRAME_PLAYBACK_EXPORT=1` runs a 1080p export of the same project (YouTube 1080p,
//! Recommended, the encoders of `CUTFRAME_ENCODERS`) on a below-normal thread on the same device
//! while playing, throttled by the transport as in the app (export.md §4.7, §10: playback keeps
//! its budget).

#![cfg(windows)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use app_core::media::MediaPath;
use app_core::{
    Clip, ClipSource, FrameRate, MediaAsset, MediaId, Project, Rgba8, Time, Track, TrackKind,
};
use arc_swap::ArcSwap;
use engine::media::readers::{ReaderOptions, ReaderPool};
use engine::media::{fingerprint, probe};
use engine::playback::pacing::{FramePicker, PREROLL_TIMEOUT, reached_end};
use engine::playback::scheduler::{self, Need};
use engine::playback::{AudioSink, EngineAudio, NoAudio, Playback, Rate, ShuttleKey, Timeline};
use engine::render::{
    Compositor, GpuContext, MissingLayers, build_plan, create_canvas, render_size,
};
use engine::transport::{SeekMode, TransportShared};

fn asset_at(path: &Path) -> MediaAsset {
    let probed = probe(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(path).expect("fingerprint"),
        info: probed.info,
    }
}

fn media_clip(media: MediaId, start: Time, duration: Time, source_in: Time) -> Clip {
    Clip {
        source: ClipSource::media_at(media, source_in),
        ..Clip::color("V", start, duration, Rgba8::BLACK)
    }
}

/// The export observer of `CUTFRAME_PLAYBACK_EXPORT`: throttled while the transport plays,
/// cancelled when playback ends.
struct BackgroundExport {
    transport: Arc<TransportShared>,
    stop: AtomicBool,
    frames: AtomicU32,
}

impl engine::export::run::ExportObserver for BackgroundExport {
    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    fn stage(&self, _: engine::export::run::ExportStage) {}
    fn set_total(&self, _: u32) {}
    fn frame_done(&self) {
        self.frames.fetch_add(1, Ordering::Relaxed);
    }
    fn playback_active(&self) -> bool {
        self.transport.is_playing()
    }
}

/// Starts the export of `project` on its own below-normal thread.
fn start_export(
    gpu: &Arc<GpuContext>,
    project: &Arc<Project>,
    observer: &Arc<BackgroundExport>,
) -> std::thread::JoinHandle<()> {
    use engine::export::encoders::{EncoderFilter, ProbeReport};
    use engine::export::plan::{
        ExportQuality, ExportResolution, VideoCodec, VideoExportSettings, plan_video_export,
    };
    let report = ProbeReport::run(&EncoderFilter::from_env());
    let settings = VideoExportSettings {
        codec: VideoCodec::H264,
        resolution: ExportResolution::P1080,
        frame_rate: None,
        quality: ExportQuality::Recommended,
        encoder: None,
    };
    let request = engine::export::run::ExportRequest {
        project: Arc::clone(project),
        plan: plan_video_export(project, &settings).expect("plan"),
        candidates: report.candidates(VideoCodec::H264, None),
        explicit_encoder: false,
        output: std::env::temp_dir().join(format!(
            "cutframe-playback-export-{}.mp4",
            std::process::id()
        )),
        overwrite: true,
    };
    eprintln!(
        "background export with {:?}",
        request.candidates.first().map(|c| c.kind)
    );
    let (gpu, observer) = (Arc::clone(gpu), Arc::clone(observer));
    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            engine::media::readers::lower_thread_priority();
            let output = request.output.clone();
            let started = Instant::now();
            let result = engine::export::run::run_export(gpu, request, &*observer);
            eprintln!(
                "background export: {} frames in {:?}, {:?}",
                observer.frames.load(Ordering::Relaxed),
                started.elapsed(),
                result.map(|s| s.encoder)
            );
            let _ = std::fs::remove_file(output);
        })
        .expect("spawn the export thread")
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn project() -> Project {
    let main = asset_at(Path::new(
        &env("CUTFRAME_PLAYBACK_MAIN").expect("CUTFRAME_PLAYBACK_MAIN"),
    ));
    let fps: u32 = env("CUTFRAME_PLAYBACK_FPS").map_or(30, |v| v.parse().expect("fps"));
    let mut project = Project::new("playback");
    project.settings.frame_rate = FrameRate::new(fps, 1).expect("frame rate");
    project.settings.width = 1920;
    project.settings.height = 1080;
    let length = main.info.duration.expect("main duration");
    let limit = env("CUTFRAME_PLAYBACK_SECONDS").map_or(length, |v| {
        Time::from_seconds(v.parse().expect("seconds")).min(length)
    });
    let main_index = project.main_index().unwrap();
    match env("CUTFRAME_PLAYBACK").as_deref().unwrap_or("main") {
        "jumpcuts" => {
            // 2 s clips whose source times jump around the file, never continuing.
            let cut = Time::from_seconds(2);
            let slots = (length.flicks() / cut.flicks()).max(2);
            let count = limit.flicks() / cut.flicks();
            for n in 0..count {
                let slot = (n * 7 + 3) % slots;
                let source_in = Time(slot * cut.flicks()).min(length - cut);
                project.tracks[main_index].clips.push(media_clip(
                    main.id,
                    Time(n * cut.flicks()),
                    cut,
                    source_in,
                ));
            }
        }
        _ => {
            let speed = env("CUTFRAME_PLAYBACK_SPEED").map_or(app_core::Speed::NORMAL, |v| {
                app_core::Speed::new(v.parse().expect("speed percent")).expect("10..=10000")
            });
            let reverse = env("CUTFRAME_PLAYBACK_REVERSE").is_some_and(|v| v == "1");
            let mut clip = media_clip(main.id, Time::ZERO, speed.timeline_floor(limit), Time::ZERO);
            clip.source = ClipSource::Media {
                media: main.id,
                source_in: Time::ZERO,
                speed,
                reverse,
            };
            eprintln!(
                "main clip: {} s at {}%, reverse {reverse}",
                clip.duration.as_seconds_f64(),
                speed.percent()
            );
            project.tracks[main_index].clips.push(clip);
            if let Some(path) = env("CUTFRAME_PLAYBACK_OVERLAY") {
                let overlay = asset_at(Path::new(&path));
                let duration = overlay.info.duration.expect("overlay duration").min(limit);
                let mut track = Track::new(TrackKind::Video, "Video 2");
                track
                    .clips
                    .push(media_clip(overlay.id, Time::ZERO, duration, Time::ZERO));
                project.tracks.insert(0, track);
                project.media.push(overlay);
            }
        }
    }
    project.media.push(main);
    project
}

#[test]
#[ignore = "needs hardware video decode and clips outside the repository"]
fn playback_drops_no_frames() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn")
            }),
        )
        .with_test_writer()
        .try_init()
        .ok();
    let gpu = Arc::new(GpuContext::new().expect("GPU"));
    eprintln!(
        "adapter: {} ({:?})",
        gpu.info().adapter,
        gpu.info().device_type
    );
    let working_set_before = working_set("WorkingSet64");
    let project = Arc::new(project());
    let snapshot = Arc::new(ArcSwap::from(Arc::clone(&project)));
    let timeline = Timeline::of(&project);
    let frame_rate = project.settings.frame_rate;
    let transport = Arc::new(TransportShared::new());
    // `CUTFRAME_PLAYBACK_AUDIO=1`: the audio engine is the clock (default device, or its null
    // output without one); else a silent wall clock.
    let audio: Arc<dyn AudioSink> = if env("CUTFRAME_PLAYBACK_AUDIO").is_some() {
        Arc::new(EngineAudio::new(Arc::clone(&transport), Arc::clone(&snapshot)).expect("audio"))
    } else {
        Arc::new(NoAudio::new(Arc::clone(&transport), Arc::clone(&snapshot)))
    };
    let playback = Playback::new(
        Arc::clone(&transport),
        snapshot,
        Arc::clone(&audio),
        Box::new(|_| {}),
    );
    let mut pool = ReaderPool::new(Some(Arc::clone(&gpu)), ReaderOptions::default(), |_| {});
    pool.set_project(&project);
    pool.set_rate(1);
    let mut compositor = Compositor::new(&gpu);
    let size = render_size(
        (project.settings.width, project.settings.height),
        (1280, 720),
        1,
    );
    let (_texture, canvas) = create_canvas(&gpu.device, size);

    let export = env("CUTFRAME_PLAYBACK_EXPORT")
        .is_some_and(|v| v == "1")
        .then(|| {
            let observer = Arc::new(BackgroundExport {
                transport: Arc::clone(&transport),
                stop: AtomicBool::new(false),
                frames: AtomicU32::new(0),
            });
            let thread = start_export(&gpu, &project, &observer);
            (observer, thread)
        });

    let vsync = Duration::from_nanos(16_666_667);
    let lead = vsync * 2;
    let mut picker = FramePicker::default();
    let mut needs: Vec<Need> = Vec::new();
    let (mut skipped, mut late, mut composites) = (0u32, 0u32, 0u32);
    let mut last: Option<i64> = None;
    let mut late_frames = Vec::new();
    let mut busy_max = Duration::ZERO;
    let mut busy_total = Duration::ZERO;
    let mut overruns = 0u32;
    let mut av_offset_max = Time::ZERO;
    let mut held = 0u32;
    let (mut ring_min, mut underruns) = (u32::MAX, 0u32);

    // Space: preroll until the first frame is exact and the ring is filled (or 300 ms).
    let space = Instant::now();
    playback.toggle();
    let seek_gen = transport.seek_generation();
    let first = build_plan(&project, Time::ZERO, size);
    loop {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let outcome = compositor.render_with(
            &gpu,
            &mut encoder,
            &first,
            &mut pool,
            &canvas,
            MissingLayers::KeepCanvas,
        );
        gpu.queue.submit([encoder.finish()]);
        compositor.submitted(&gpu);
        let ready = outcome.stale_layers + outcome.missing_layers == 0 && audio.ring_ready();
        if ready || space.elapsed() > PREROLL_TIMEOUT {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let preroll = space.elapsed();
    playback.preroll_done(seek_gen);
    let mut motion = None;

    let start = Instant::now();
    let mut submission = None;
    for n in 1u32.. {
        let deadline = start + vsync * n;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        } else {
            overruns += 1;
        }
        let began = Instant::now();
        let Some(audible) = audio.audible().filter(|a| a.seek_gen == seek_gen) else {
            // Started, the first sample is not audible yet: hold the first frame.
            held += 1;
            assert!(n < 120, "the clock never started");
            continue;
        };
        motion.get_or_insert_with(|| space.elapsed());
        if reached_end(audible, Rate::FORWARD, timeline, false) {
            playback.end(seek_gen);
            break;
        }
        // The swap chain's frame latency: the previous frame's GPU work is done.
        if let Some(index) = submission.take() {
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .expect("poll");
        }
        compositor.release_completed();
        let polled = began.elapsed();
        let pick = picker.pick(audible, lead, Rate::FORWARD, timeline, false);
        if pick.t_show < timeline.duration {
            av_offset_max = av_offset_max.max(Time((pick.time - pick.t_show).flicks().abs()));
        }
        if n % 300 == 0 {
            let stats = audio.stats();
            ring_min = ring_min.min(stats.ring_min_fill);
            underruns = underruns.max(stats.underruns);
        }
        let plan = build_plan(&project, pick.time, size);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let composing = Instant::now();
        let outcome = compositor.render_with(
            &gpu,
            &mut encoder,
            &plan,
            &mut pool,
            &canvas,
            MissingLayers::KeepCanvas,
        );
        submission = Some(gpu.queue.submit([encoder.finish()]));
        compositor.submitted(&gpu);
        let composed = composing.elapsed();
        let mut primed = Duration::ZERO;
        if last != Some(pick.frame) {
            composites += 1;
            if let Some(last) = last
                && pick.frame > last + 1
            {
                skipped += (pick.frame - last - 1) as u32;
            }
            if outcome.stale_layers + outcome.missing_layers > 0 {
                late += 1;
                late_frames.push(pick.frame);
            }
            last = Some(pick.frame);
            scheduler::lookahead(&project, pick.time, Rate::FORWARD, false, &mut needs);
            let priming = Instant::now();
            for need in &needs {
                pool.prime(need.media, need.sample_time, need.pace);
            }
            primed = priming.elapsed();
        }
        let busy = began.elapsed();
        if busy > Duration::from_millis(20) {
            eprintln!(
                "slow vsync {n}: total {busy:?}, poll {polled:?}, composite {composed:?}, prime {primed:?}, frame {}",
                pick.frame
            );
        }
        busy_total += busy;
        busy_max = busy_max.max(busy);
    }
    if let Some((observer, thread)) = export {
        eprintln!(
            "background export: {} frames during playback",
            observer.frames.load(Ordering::Relaxed)
        );
        observer.stop.store(true, Ordering::Relaxed);
        thread.join().expect("the export thread");
    }
    let frames = timeline.last_frame() + 1;
    underruns = underruns.max(audio.stats().underruns);
    eprintln!(
        "preroll {preroll:?}, space to motion {motion:?}, vsyncs held for the first sample {held};          av_offset_max {:.1} ms; underruns {}, ring_min_fill {}",
        av_offset_max.as_seconds_f64() * 1000.0,
        underruns,
        ring_min
    );
    let dropped = skipped + late;
    eprintln!(
        "frames {frames} at {}/{} fps, composited {composites}, dropped {dropped} ({:.2} %): skipped {skipped}, late {late}",
        frame_rate.num(),
        frame_rate.den(),
        100.0 * f64::from(dropped) / frames as f64
    );
    eprintln!(
        "vsync step: avg {:.2} ms, max {:.2} ms, overruns {overruns}; readers {}, seeks {}",
        busy_total.as_secs_f64() * 1000.0 / f64::from(composites.max(1)),
        busy_max.as_secs_f64() * 1000.0,
        pool.reader_count(),
        project
            .media
            .iter()
            .map(|m| pool.seek_count(m.id))
            .sum::<u32>(),
    );
    for media in &project.media {
        eprintln!("{}: {:?}", media.path.absolute, pool.decode_path(media.id));
    }
    let peak = working_set("PeakWorkingSet64");
    eprintln!(
        "working set: {} MB before, peak {} MB (growth {} MB); reverse readers {}",
        working_set_before / 1_000_000,
        peak / 1_000_000,
        peak.saturating_sub(working_set_before) / 1_000_000,
        pool.reverse_readers()
    );
    if !late_frames.is_empty() {
        eprintln!(
            "late frames: {:?}",
            &late_frames[..late_frames.len().min(40)]
        );
    }
    let budget: f64 = env("CUTFRAME_PLAYBACK_BUDGET").map_or(0.0, |v| v.parse().expect("budget"));
    assert!(
        f64::from(dropped) <= budget / 100.0 * frames as f64,
        "dropped {dropped} of {frames}"
    );
}

/// A memory counter of this process (`WorkingSet64`, `PeakWorkingSet64`) in bytes, read with
/// PowerShell outside the timed loop; 0 when it cannot be read.
fn working_set(counter: &str) -> u64 {
    let script = format!("(Get-Process -Id {}).{counter}", std::process::id());
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse().ok())
        .unwrap_or(0)
}

/// Main-thread time per transport command (playback.md §11: <= 0.1 ms).
#[test]
#[ignore = "a timing measurement; run with --release"]
fn transport_commands_are_cheap() {
    let mut project = Project::new("t");
    let main = project.main_index().unwrap();
    project.tracks[main].clips.push(Clip::color(
        "C",
        Time::ZERO,
        Time::from_seconds(600),
        Rgba8::BLACK,
    ));
    let snapshot = Arc::new(ArcSwap::from_pointee(project));
    let shared = Arc::new(TransportShared::new());
    let audio = Arc::new(NoAudio::new(Arc::clone(&shared), Arc::clone(&snapshot)));
    let (tx, rx) = crossbeam_channel::unbounded();
    let playback = Playback::new(
        shared,
        snapshot,
        audio,
        Box::new(move |wake| {
            let _ = tx.send(wake);
        }),
    );
    let rounds = 20_000u32;
    let mut worst = Duration::ZERO;
    let started = Instant::now();
    for n in 0..rounds {
        let begin = Instant::now();
        match n % 6 {
            0 => playback.toggle(),
            1 => playback.shuttle(ShuttleKey::Forward),
            2 => {
                playback.seek(Time::from_seconds(i64::from(n % 500)), SeekMode::Exact);
            }
            3 => playback.set_loop(n % 2 == 0),
            4 => {
                playback.step(1);
            }
            _ => playback.pause(),
        }
        worst = worst.max(begin.elapsed());
        while rx.try_recv().is_ok() {}
    }
    let average = started.elapsed() / rounds;
    eprintln!("transport command: avg {average:?}, worst {worst:?}");
    assert!(average < Duration::from_micros(100));
}
