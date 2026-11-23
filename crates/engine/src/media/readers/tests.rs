//! Pool and reader-thread tests on the committed fixtures (software decode, no GPU).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use app_core::media::MediaPath;
use app_core::{MediaAsset, MediaId, Project};

use super::*;

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
    pool.set_project(Arc::new(project));
    (pool, ready)
}

fn ms(ms: i64) -> Time {
    Time::from_flicks(ms * app_core::FLICKS_PER_SECOND / 1000)
}

fn exact(pool: &mut ReaderPool, media: MediaId, t: Time) -> crate::render::frames::FrameRef {
    match pool.video(media, t, FetchPolicy::Exact) {
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
    let first = pool.video(clip.id, ms(500), FetchPolicy::BestAvailable);
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
    let again = pool.video(clip.id, ms(500), FetchPolicy::BestAvailable);
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
        pool.video(c.id, ms(100), FetchPolicy::BestAvailable),
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
    pool.set_project(Arc::new(project));
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
