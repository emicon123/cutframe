//! Pool and reader-thread tests on the committed fixtures (software decode, no GPU).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use app_core::media::MediaPath;
use app_core::{MediaAsset, MediaId, Project};

use super::*;
use crate::render::frames::Pace;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media")
        .join(name)
}

fn asset(name: &str) -> MediaAsset {
    let path = fixture(name);
    let probed = crate::media::probe(&path).expect("probe");
    MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: crate::media::fingerprint(&path).expect("fingerprint"),
        info: probed.info,
    }
}

fn pool_with(media: &[MediaAsset], options: ReaderOptions) -> (ReaderPool, Arc<AtomicUsize>) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init()
        .ok();
    let ready = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ready);
    let mut pool = ReaderPool::new(None, options, move |event| {
        if event == ReaderEvent::FrameReady {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut project = Project::new("test");
    project.media = media.to_vec();
    pool.set_project(&project);
    (pool, ready)
}

fn ms(ms: i64) -> Time {
    Time::from_flicks(ms * app_core::FLICKS_PER_SECOND / 1000)
}

fn exact(pool: &mut ReaderPool, media: MediaId, t: Time) -> crate::render::frames::FrameRef {
    match pool.video(media, t, Pace::NORMAL, FetchPolicy::Exact) {
        FrameLookup::Exact(frame) => frame,
        other => panic!("expected an exact frame at {t:?}, got {other:?}"),
    }
}

#[test]
fn progressive_scrub_publishes_the_keyframe_before_the_exact_frame() {
    // One GOP from frame 0: a seek to 0.8 s decodes forward from the keyframe at 0.
    let clip = asset("h264-aac.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    pool.begin_frame();
    let frame = exact(&mut pool, clip.id, ms(800));
    let pts = frame.pts().expect("a video frame");
    assert!(pts <= ms(800) && pts > ms(700), "{pts:?}");
    let published = pool.readers[0].published();
    assert_eq!(
        published.first(),
        Some(&Time::ZERO),
        "the keyframe came first"
    );
    assert!(published.len() > 20, "every frame on the way was published");
    assert!(published.windows(2).all(|w| w[0] < w[1]));
    assert!(published.contains(&pts));
}

#[test]
fn frame_ready_is_sent_once_until_the_next_wait() {
    let clip = asset("h264-aac.mp4");
    let (mut pool, ready) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    pool.begin_frame();
    let first = pool.video(clip.id, ms(500), Pace::NORMAL, FetchPolicy::BestAvailable);
    assert!(
        !matches!(first, FrameLookup::Exact(_)),
        "nothing is decoded yet"
    );
    let start = Instant::now();
    while ready.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // The paused reader keeps decoding up to 2 frames ahead; that sends nothing more.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(ready.load(Ordering::SeqCst), 1, "coalesced");
    pool.begin_frame();
    let again = pool.video(clip.id, ms(500), Pace::NORMAL, FetchPolicy::BestAvailable);
    assert!(matches!(again, FrameLookup::Exact(_)), "{again:?}");
}

#[test]
fn readers_are_limited_shared_and_reassigned() {
    let a = asset("h264-aac.mp4");
    let b = asset("vp8.webm");
    let c = asset("mpeg4.m4v");
    let media = [a.clone(), b.clone(), c.clone()];
    let options = ReaderOptions {
        max_readers: 2,
        max_hw_readers: 0,
        ..ReaderOptions::default()
    };
    let (mut pool, _) = pool_with(&media, options);

    pool.begin_frame();
    exact(&mut pool, a.id, ms(100));
    // A second layer of the same frame at the same time shares the reader.
    exact(&mut pool, a.id, ms(100));
    assert_eq!(pool.reader_count(), 1);
    exact(&mut pool, b.id, ms(100));
    assert_eq!(pool.reader_count(), 2);
    // At the limit with every reader busy in this frame: no reader for C yet.
    assert!(matches!(
        pool.video(c.id, ms(100), Pace::NORMAL, FetchPolicy::BestAvailable),
        FrameLookup::Missing
    ));

    // Next frame: C takes the least recently used reader (A's).
    pool.begin_frame();
    exact(&mut pool, b.id, ms(133));
    exact(&mut pool, c.id, ms(100));
    assert_eq!(pool.reader_count(), 2);
    assert!(
        pool.decode_path(a.id).is_none(),
        "A's reader was reassigned"
    );
    assert_eq!(pool.decode_path(c.id), Some(DecodePath::Software));
}

#[test]
fn two_layers_of_one_media_at_different_times_get_two_readers() {
    let clip = asset("h264-aac.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    let frame = Time::from_flicks(app_core::FLICKS_PER_SECOND / 30);
    for n in 0..5 {
        pool.begin_frame();
        let step = Time::from_flicks(frame.flicks() * n);
        exact(&mut pool, clip.id, ms(100) + step);
        exact(&mut pool, clip.id, ms(600) + step);
    }
    assert_eq!(pool.reader_count(), 2, "one reader per layer, no ping-pong");
}

#[test]
fn idle_readers_close_after_five_seconds() {
    let clip = asset("h264-aac.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    pool.begin_frame();
    exact(&mut pool, clip.id, ms(100));
    assert_eq!(pool.reader_count(), 1);
    pool.begin_frame();
    assert_eq!(pool.reader_count(), 1, "used a moment ago");
    pool.readers[0].last_used -= IDLE_CLOSE;
    pool.begin_frame();
    assert_eq!(pool.reader_count(), 0);
}

#[test]
fn relinked_media_get_a_new_reader_and_new_frame_ids() {
    let mut clip = asset("h264-aac.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    pool.begin_frame();
    let before = exact(&mut pool, clip.id, ms(100)).id();
    clip.path = MediaPath::absolute(fixture("mpeg4.m4v").to_string_lossy());
    let mut project = Project::new("relinked");
    project.media = vec![clip.clone()];
    pool.set_project(&project);
    assert_eq!(pool.reader_count(), 0, "the old file's reader closed");
    pool.begin_frame();
    let after = exact(&mut pool, clip.id, ms(100)).id();
    assert_eq!(after.media, before.media);
    assert_ne!(after.source, before.source);
}

#[test]
fn offline_media_maps_to_the_offline_placeholder() {
    let kind = |kind| placeholder_for(&MediaError::new(kind, "x"));
    assert_eq!(kind(MediaErrorKind::FileNotFound), PlaceholderKind::Offline);
    assert_eq!(kind(MediaErrorKind::AccessDenied), PlaceholderKind::Offline);
    assert_eq!(
        kind(MediaErrorKind::UnsupportedPath),
        PlaceholderKind::Offline
    );
    assert_eq!(kind(MediaErrorKind::Damaged), PlaceholderKind::DecodeError);
    assert_eq!(
        kind(MediaErrorKind::UnsupportedCodec),
        PlaceholderKind::DecodeError
    );
}

/// A project whose main track plays `clip` as (timeline start, duration, source in) in ms.
fn edit(media: &MediaAsset, clips: &[(i64, i64, i64)]) -> Project {
    let mut project = Project::new("edit");
    project.media = vec![media.clone()];
    let main = project.main_index().expect("main track");
    for &(start, duration, source_in) in clips {
        project.tracks[main].clips.push(app_core::Clip {
            source: app_core::ClipSource::media_at(media.id, ms(source_in)),
            ..app_core::Clip::color("V", ms(start), ms(duration), app_core::Rgba8::BLACK)
        });
    }
    project
}

/// Plays `project` frame by frame like the render thread: the lookups of the frame's plan,
/// then the scheduler's lookahead primes readers. `policy` for the lookups; `pace` between
/// frames. Returns the lookups of each frame.
fn play(
    pool: &mut ReaderPool,
    project: &Project,
    frames: std::ops::Range<i64>,
    policy: FetchPolicy,
    pace: Duration,
) -> Vec<FrameLookup> {
    use crate::playback::{Rate, scheduler};
    use crate::render::{LayerContent, build_plan};
    pool.set_project(project);
    pool.set_rate(1);
    let rate = project.settings.frame_rate;
    let mut needs = Vec::new();
    let mut lookups = Vec::new();
    for n in frames {
        let t = Time::from_frames(n, rate);
        let plan = build_plan(project, t, (64, 64));
        pool.begin_frame();
        for layer in &plan.layers {
            if let LayerContent::Video {
                media,
                sample_time,
                pace,
                ..
            } = layer.content
            {
                lookups.push(pool.video(media, sample_time, pace, policy));
            }
        }
        scheduler::lookahead(project, t, Rate::FORWARD, false, &mut needs);
        for need in &needs {
            pool.prime(need.media, need.sample_time, need.pace);
        }
        std::thread::sleep(pace);
    }
    lookups
}

#[test]
fn a_split_point_plays_without_a_seek() {
    let clip = asset("h264-aac.mp4");
    // Split at 400 ms: the second half continues the source.
    let project = edit(&clip, &[(0, 400, 100), (400, 500, 500)]);
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    let lookups = play(
        &mut pool,
        &project,
        0..27,
        FetchPolicy::Exact,
        Duration::ZERO,
    );
    assert_eq!(lookups.len(), 27);
    assert_eq!(pool.reader_count(), 1, "one reader across the split");
    assert_eq!(pool.seek_count(clip.id), 1, "only the first frame seeks");
}

#[test]
fn a_jump_cut_is_primed_before_it_starts() {
    let clip = asset("h264-aac.mp4");
    // A backwards jump cut at 300 ms: source 500..800 ms, then 0..300 ms.
    let project = edit(&clip, &[(0, 300, 500), (300, 300, 0)]);
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    // The frames before the cut, at about real time.
    play(
        &mut pool,
        &project,
        0..9,
        FetchPolicy::Exact,
        Duration::from_millis(30),
    );
    assert_eq!(pool.reader_count(), 2, "a second reader for the jump cut");
    let before = pool.seek_count(clip.id);
    assert_eq!(before, 2, "the first frame and the primed reader");
    // The first frame after the cut is ready at once, without waiting.
    let after = play(
        &mut pool,
        &project,
        9..12,
        FetchPolicy::BestAvailable,
        Duration::ZERO,
    );
    for lookup in &after {
        assert!(matches!(lookup, FrameLookup::Exact(_)), "{lookup:?}");
    }
    assert_eq!(pool.seek_count(clip.id), before, "no seek at the cut");
}

#[test]
fn a_slow_reader_catches_up() {
    let clip = asset("h264-aac.mp4");
    let project = edit(&clip, &[(0, 1000, 0)]);
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    // 30 ms per decoded frame while playback asks for a new frame every 5 ms: the reader
    // falls behind, discards non-reference frames, then jumps ahead.
    pool.decode_delay = Duration::from_millis(30);
    pool.set_project(&project);
    pool.set_rate(1);
    let mut discarded = false;
    let frame = Time::from_flicks(app_core::FLICKS_PER_SECOND / 30);
    for n in 0..28 {
        pool.begin_frame();
        let t = Time::from_flicks(frame.flicks() * n + frame.flicks() / 2);
        pool.video(clip.id, t, Pace::NORMAL, FetchPolicy::BestAvailable);
        discarded |= pool.readers[0].discard() == queue::Discard::NonRef;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        discarded,
        "non-reference frames were discarded while behind"
    );
    assert!(
        pool.catch_up_count(clip.id) >= 1,
        "jumped ahead when far behind"
    );
    assert_eq!(pool.reader_count(), 1);
}

#[test]
fn keyframes_only_decodes_only_keyframes() {
    let clip = asset("h264-aac.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    // Shuttle at 4x (`Exact` lookups would decode every frame).
    pool.set_rate(4);
    // The fixture has one keyframe at 0: every later time holds it.
    let start = Instant::now();
    let frame = loop {
        pool.begin_frame();
        match pool.video(clip.id, ms(500), Pace::NORMAL, FetchPolicy::BestAvailable) {
            FrameLookup::Exact(frame) => break frame,
            other => assert!(start.elapsed() < Duration::from_secs(10), "{other:?}"),
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(frame.pts(), Some(Time::ZERO));
    assert_eq!(pool.readers[0].published(), vec![Time::ZERO]);
}

// ---- GOP-reverse mode (`clip-speed-audio.md` §5.2) ------------------------------------------

const REVERSE: Pace = Pace::Reverse {
    speed: app_core::Speed::NORMAL,
};

const HALF_SOURCE_FRAME: Time = Time::from_flicks(app_core::FLICKS_PER_SECOND / 60);

/// Media time of the centre of frame `n` of a 30 fps source.
fn centre(n: i64) -> Time {
    Time::from_flicks(n * app_core::FLICKS_PER_SECOND / 30) + HALF_SOURCE_FRAME
}

fn lookup(pool: &mut ReaderPool, media: MediaId, t: Time, pace: Pace) -> FrameLookup {
    pool.begin_frame();
    pool.video(media, t, pace, FetchPolicy::Exact)
}

fn pts_of(lookup: &FrameLookup) -> Time {
    match lookup {
        FrameLookup::Exact(frame) => frame.pts().expect("a video frame"),
        other => panic!("expected an exact frame, got {other:?}"),
    }
}

#[test]
fn reverse_mode_with_a_tiny_window_returns_the_frames_of_forward_decoding() {
    // Keyframes at 0, 12, 21, 30, 39, 48, 57; room for 3 frames per window, so every GOP is
    // decoded several times over.
    let clip = asset("numbered-gop.mp4");
    let (mut forward, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    let (mut reverse, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    reverse.set_rate(1);
    let frame_bytes = match lookup(&mut forward, clip.id, centre(0), Pace::NORMAL) {
        FrameLookup::Exact(frame) => frame.data().cpu_bytes(),
        other => panic!("{other:?}"),
    };
    reverse.window_bytes = 3 * frame_bytes;
    let mut max_bytes = 0;
    for n in (0..60).rev() {
        let expected = pts_of(&lookup(&mut forward, clip.id, centre(n), Pace::NORMAL));
        let got = pts_of(&lookup(&mut reverse, clip.id, centre(n), REVERSE));
        assert_eq!(got, expected, "frame {n}");
        assert_eq!(got, centre(n) - HALF_SOURCE_FRAME, "frame {n}");
        max_bytes = max_bytes.max(reverse.readers[0].window_bytes());
    }
    assert_eq!(reverse.reader_count(), 1);
    assert_eq!(reverse.reverse_readers(), 1);
    assert!(
        max_bytes <= 2 * 3 * frame_bytes,
        "two windows at most: {max_bytes} bytes"
    );
    // 60 frames in windows of 3: about 20 windows, each its own seek.
    let seeks = reverse.seek_count(clip.id);
    assert!((20..=24).contains(&seeks), "{seeks} seeks");
    // Each window decoded forward from its keyframe: frames below the window were decoded
    // again (more frames published than shown).
    assert!(reverse.readers[0].published().len() > 60);
}

#[test]
fn reverse_playback_crosses_gop_boundaries_without_waiting() {
    let clip = asset("numbered-gop.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    pool.set_rate(1);
    // The first frame waits for its window; the next earlier windows are prepared meanwhile.
    assert_eq!(
        pts_of(&lookup(&mut pool, clip.id, centre(59), REVERSE)),
        centre(59) - HALF_SOURCE_FRAME
    );
    let mut late = Vec::new();
    for n in (0..59).rev() {
        std::thread::sleep(Duration::from_millis(15));
        pool.begin_frame();
        match pool.video(clip.id, centre(n), REVERSE, FetchPolicy::BestAvailable) {
            FrameLookup::Exact(frame) => {
                assert_eq!(
                    frame.pts(),
                    Some(centre(n) - HALF_SOURCE_FRAME),
                    "frame {n}"
                );
            }
            _ => late.push(n),
        }
    }
    assert!(late.is_empty(), "frames not ready in time: {late:?}");
    // One seek per GOP (7) and two looks below the first frame (they land on it twice).
    let seeks = pool.seek_count(clip.id);
    assert!(seeks <= 9, "{seeks} seeks");
}

/// `project` with its main-track clips reversed at `speed` percent.
fn reversed(mut project: Project, speed: u16) -> Project {
    let main = project.main_index().expect("main track");
    for clip in &mut project.tracks[main].clips {
        if let app_core::ClipSource::Media {
            speed: clip_speed,
            reverse,
            ..
        } = &mut clip.source
        {
            *clip_speed = app_core::Speed::new(speed).expect("speed");
            *reverse = true;
        }
    }
    project
}

#[test]
fn a_split_reversed_clip_plays_on_one_reverse_reader() {
    let clip = asset("numbered-gop.mp4");
    // Source 100..1900 ms reversed, whole and split at 900 ms (the halves as a split makes
    // them: the left half shows the top of the range).
    let whole = reversed(edit(&clip, &[(0, 1800, 100)]), 100);
    let split = reversed(edit(&clip, &[(0, 900, 1000), (900, 900, 100)]), 100);
    let mut seeks = Vec::new();
    let mut shown = Vec::new();
    for project in [&whole, &split] {
        let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
        let lookups = play(
            &mut pool,
            project,
            0..54,
            FetchPolicy::Exact,
            Duration::ZERO,
        );
        assert_eq!(pool.reader_count(), 1, "one reader across the split");
        assert_eq!(pool.reverse_readers(), 1);
        seeks.push(pool.seek_count(clip.id));
        shown.push(lookups.iter().map(pts_of).collect::<Vec<_>>());
    }
    assert_eq!(shown[0], shown[1], "the same frames");
    assert!(shown[0].windows(2).all(|w| w[0] > w[1]), "downwards");
    // Window seeks depend a little on timing (how far the next window is prepared when the
    // lookup arrives), so two separate runs can differ by one window seek under load (seen as
    // [7, 8] in a full `cargo test --workspace`). The split itself adds none: the one-reader and
    // same-frames checks above are the real guarantee; this only catches a seek per split.
    assert!(
        seeks[1] <= seeks[0] + 1,
        "the split costs no seek: {seeks:?}"
    );
}

#[test]
fn reverse_readers_are_bound_by_direction_and_limited_to_two() {
    let a = asset("numbered-gop.mp4");
    let b = asset("h264-aac.mp4");
    let c = asset("mpeg4.m4v");
    let (mut pool, _) = pool_with(&[a.clone(), b.clone(), c.clone()], ReaderOptions::default());
    pool.set_rate(1);
    pool.begin_frame();
    pts_of(&pool.video(a.id, ms(500), Pace::NORMAL, FetchPolicy::Exact));
    pts_of(&pool.video(a.id, ms(500), REVERSE, FetchPolicy::Exact));
    assert_eq!(
        pool.reader_count(),
        2,
        "a forward and a reverse reader of one media"
    );
    pts_of(&pool.video(b.id, ms(500), REVERSE, FetchPolicy::Exact));
    assert_eq!(pool.reverse_readers(), 2);
    // A third reversed media in the next frame takes the least recently used reverse reader.
    pool.begin_frame();
    pts_of(&pool.video(b.id, ms(466), REVERSE, FetchPolicy::Exact));
    pts_of(&pool.video(c.id, ms(300), REVERSE, FetchPolicy::Exact));
    assert_eq!(pool.reverse_readers(), 2);
    assert_eq!(pool.reader_count(), 3);
    assert!(
        pool.readers.iter().all(|r| !r.reverse || r.media() != a.id),
        "the reverse reader of A was reassigned"
    );
}

#[test]
fn the_effective_rate_switches_to_keyframes_only_above_4x() {
    let clip = asset("numbered-gop.mp4");
    let (mut pool, _) = pool_with(std::slice::from_ref(&clip), ReaderOptions::default());
    let at = |speed: u16| Pace::Forward {
        speed: app_core::Speed::new(speed).unwrap(),
    };
    let best = FetchPolicy::BestAvailable;
    let steering = |pool: &ReaderPool, pace| pool.steering(best, pace).1;
    pool.set_rate(1);
    assert!(!steering(&pool, at(400)).keyframes_only, "4x: every frame");
    assert!(steering(&pool, at(400)).catch_up);
    assert!(steering(&pool, at(401)).keyframes_only);
    pool.set_rate(2);
    assert!(steering(&pool, at(200)).catch_up, "2x of 2x");
    assert!(steering(&pool, at(250)).keyframes_only, "5x");
    assert!(
        !pool
            .steering(FetchPolicy::Exact, at(10_000))
            .1
            .keyframes_only,
        "export is exact"
    );
    pool.set_rate(4);
    assert!(
        steering(&pool, at(50)).keyframes_only,
        "the shuttle rule stays"
    );
    pool.set_rate(0);
    assert!(!steering(&pool, at(10_000)).keyframes_only, "paused");
    pool.set_rate(1);
    assert_eq!(
        steering(&pool, Pace::Still).mode,
        Mode::Paused,
        "a freeze frame decodes nothing ahead"
    );
    assert!(pool.steering(best, REVERSE).0, "a reverse reader");
    assert!(!pool.steering(best, REVERSE).1.catch_up);
    // Reverse transport over a reversed clip: the source moves forward.
    pool.set_rate(-1);
    let (reverse, steering_back) = pool.steering(best, REVERSE);
    assert!(!reverse);
    assert_eq!(steering_back.mode, Mode::Playing);
    let (reverse, scrub) = pool.steering(best, Pace::NORMAL);
    assert!(!reverse);
    assert_eq!(
        scrub.mode,
        Mode::Paused,
        "J over a forward clip stays a timed scrub"
    );
}
