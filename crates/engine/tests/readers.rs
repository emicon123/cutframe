//! The reader pool as the compositor uses it (render-graph.md §4.1, §5.3, §10), on the committed
//! fixtures (`tests/fixtures/media/README.md`) with software decode: exact frames by sample and
//! hold (VFR included), offline media, and image layers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use app_core::media::{MediaPath, Rotation};
use app_core::{FLICKS_PER_SECOND, MediaAsset, MediaId, Project, Time};
use engine::media::decode::{frame_index_at, video_frame_times};
use engine::media::readers::{DecodePath, ReaderEvent, ReaderOptions, ReaderPool};
use engine::media::{CpuFrame, PlaneLayout, SampleDepth, fingerprint, probe};
use engine::render::PlaceholderKind;
use engine::render::{AdapterMode, FetchPolicy, FrameLookup, FrameProvider, FrameRef, GpuContext};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media")
        .join(name)
}

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

type Events = Arc<Mutex<Vec<ReaderEvent>>>;

fn pool(gpu: Option<Arc<GpuContext>>, media: Vec<MediaAsset>) -> (ReaderPool, Events) {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let mut pool = ReaderPool::new(gpu, ReaderOptions::default(), move |event| {
        sink.lock().unwrap().push(event);
    });
    let mut project = Project::new("readers");
    project.media = media;
    pool.set_project(Arc::new(project));
    (pool, events)
}

fn exact_video(pool: &mut ReaderPool, media: MediaId, t: Time) -> FrameRef {
    pool.begin_frame();
    match pool.video(media, t, FetchPolicy::Exact) {
        FrameLookup::Exact(frame) => frame,
        other => panic!("expected an exact frame at {t:?}, got {other:?}"),
    }
}

fn exact_image(pool: &mut ReaderPool, media: MediaId) -> FrameRef {
    pool.begin_frame();
    match pool.image(media, FetchPolicy::Exact) {
        FrameLookup::Exact(frame) => frame,
        other => panic!("expected an image, got {other:?}"),
    }
}

/// Sample times across a clip: before the first frame, on frames, between frames, in a VFR gap
/// and past the end (where the last frame holds).
fn sample_times(times: &[Time]) -> Vec<Time> {
    let mut samples = vec![Time::ZERO];
    for w in times.windows(2) {
        samples.push(w[0]);
        samples.push(Time::from_flicks((w[0].flicks() + w[1].flicks()) / 2));
    }
    let last = *times.last().unwrap();
    samples.push(last);
    samples.push(last + Time::from_seconds(5));
    samples
}

fn check_exact_frames(name: &str, gpu: Option<Arc<GpuContext>>) {
    let path = fixture(name);
    let asset = asset_at(&path);
    let times = video_frame_times(&path, &asset.info).expect("frame times");
    let (mut pool, _) = pool(gpu, vec![asset.clone()]);
    // Forward (no seeks within 1 s), then backwards (a seek per lookup).
    let mut samples = sample_times(&times);
    let backwards: Vec<Time> = samples.iter().rev().step_by(7).copied().collect();
    samples.extend(backwards);
    for t in samples {
        let frame = exact_video(&mut pool, asset.id, t);
        let index = frame_index_at(&times, t).unwrap_or(0);
        assert_eq!(frame.pts(), Some(times[index]), "{name} at {t:?}");
        assert_eq!(frame.id().media, asset.id);
        let cpu = frame.cpu().expect("software decode gives CPU frames");
        cpu_frame_is_sane(cpu, asset.info.video.as_ref().unwrap().width);
    }
    assert_eq!(pool.decode_path(asset.id), Some(DecodePath::Software));
}

fn cpu_frame_is_sane(frame: &CpuFrame, width: u32) {
    assert_eq!(frame.visible.0, width);
    assert!(
        frame.planes[0]
            .as_ref()
            .is_some_and(|p| !p.bytes.is_empty())
    );
}

#[test]
fn exact_frames_follow_sample_and_hold_including_vfr() {
    // 50 frames at 30 fps with frames 10-19 dropped: one frame lasts 11/30 s.
    check_exact_frames("hevc-portrait-cw90-vfr.mov", None);
    check_exact_frames("h264-aac.mp4", None);
}

#[test]
fn every_video_fixture_decodes_in_software_on_warp() {
    let gpu = Arc::new(GpuContext::new_with(AdapterMode::SoftwareOnly).expect("WARP"));
    for name in [
        "h264-aac.mp4",
        "hevc-portrait-cw90-vfr.mov",
        "hevc-hlg-dv84.mov",
        "av1-10bit-pq.mp4",
        "av1.mkv",
        "vp8.webm",
        "vp9-opus-no-duration.webm",
        "prores.mov",
        "mjpeg.avi",
        "mpeg4.m4v",
        "mpeg2-anamorphic.mpg",
        "animated.gif",
        "two-audio-tracks.mkv",
        "crashed-recording.mkv",
        "ąęłó/zażółć.mp4",
    ] {
        let asset = asset_at(&fixture(name));
        let (mut pool, _) = pool(Some(Arc::clone(&gpu)), vec![asset.clone()]);
        let frame = exact_video(
            &mut pool,
            asset.id,
            Time::from_flicks(FLICKS_PER_SECOND / 3),
        );
        assert!(
            frame.cpu().is_some(),
            "{name}: WARP has no hardware decoder"
        );
        assert_eq!(
            pool.decode_path(asset.id),
            Some(DecodePath::Software),
            "{name}"
        );
    }
}

#[test]
fn a_missing_file_is_offline_and_reported_once() {
    let mut asset = asset_at(&fixture("h264-aac.mp4"));
    asset.path = MediaPath::absolute(fixture("gone.mp4").to_string_lossy());
    let (mut pool, events) = pool(None, vec![asset.clone()]);
    for _ in 0..3 {
        pool.begin_frame();
        assert!(matches!(
            pool.video(asset.id, Time::ZERO, FetchPolicy::Exact),
            FrameLookup::Failed(PlaceholderKind::Offline)
        ));
    }
    // A missing image too, and a media id the project does not know.
    let mut image = asset_at(&fixture("image.png"));
    image.path = MediaPath::absolute(fixture("gone.png").to_string_lossy());
    let mut project = Project::new("readers");
    project.media = vec![asset.clone(), image.clone()];
    pool.set_project(Arc::new(project));
    for _ in 0..2 {
        pool.begin_frame();
        assert!(matches!(
            pool.image(image.id, FetchPolicy::Exact),
            FrameLookup::Failed(PlaceholderKind::Offline)
        ));
    }
    assert!(matches!(
        pool.video(MediaId::new(), Time::ZERO, FetchPolicy::BestAvailable),
        FrameLookup::Failed(PlaceholderKind::Offline)
    ));
    let unavailable: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            ReaderEvent::MediaUnavailable(id) => Some(*id),
            ReaderEvent::FrameReady => None,
        })
        .collect();
    assert_eq!(unavailable, vec![asset.id, image.id], "once per media");
}

#[test]
fn best_available_never_blocks_and_reaches_the_exact_frame() {
    let asset = asset_at(&fixture("h264-aac.mp4"));
    let (mut pool, _) = pool(None, vec![asset.clone()]);
    let t = Time::from_flicks(FLICKS_PER_SECOND / 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let frame = loop {
        pool.begin_frame();
        match pool.video(asset.id, t, FetchPolicy::BestAvailable) {
            FrameLookup::Exact(frame) => break frame,
            FrameLookup::Stale(frame) => assert!(frame.pts().unwrap() <= t),
            FrameLookup::Missing => {}
            FrameLookup::Failed(kind) => panic!("{kind:?}"),
        }
        assert!(std::time::Instant::now() < deadline, "no exact frame");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    let exact = exact_video(&mut pool, asset.id, t);
    assert_eq!(frame.id(), exact.id());
}

/// An image's frame: packed RGBA8, straight alpha, stored orientation.
fn rgba(frame: &FrameRef) -> &CpuFrame {
    let cpu = frame.cpu().expect("images are CPU frames");
    assert_eq!(cpu.layout, PlaneLayout::Packed);
    assert_eq!(cpu.depth, SampleDepth::U8);
    assert_eq!(frame.pts(), None);
    cpu
}

fn alpha_at(cpu: &CpuFrame, x: u32, y: u32) -> u8 {
    let plane = cpu.planes[0].as_ref().unwrap();
    plane.bytes[y as usize * plane.stride + x as usize * 4 + 3]
}

/// The size after the stream's rotation, as the convert pass turns it.
fn upright(size: (u32, u32), rotation: Rotation) -> (u32, u32) {
    if rotation.swaps_axes() {
        (size.1, size.0)
    } else {
        size
    }
}

#[test]
fn images_decode_once_with_alpha_and_upright_size() {
    let alpha = asset_at(&fixture("image-alpha.png"));
    let exif = asset_at(&fixture("exif-orientation-6.jpg"));
    let heic = asset_at(&fixture("image-grid.heic"));
    let webp = asset_at(&fixture("image.webp"));
    let (mut pool, _) = pool(
        None,
        vec![alpha.clone(), exif.clone(), heic.clone(), webp.clone()],
    );

    let frame = exact_image(&mut pool, alpha.id);
    let cpu = rgba(&frame);
    assert_eq!(cpu.visible, (64, 48));
    assert_eq!(alpha_at(cpu, 0, 10), 0, "transparent left column");
    assert_eq!(alpha_at(cpu, 63, 10), 255, "opaque right column");
    assert!(
        (120..=135).contains(&alpha_at(cpu, 32, 10)),
        "straight alpha ramp"
    );
    // Decoded once: the same frame again.
    assert_eq!(exact_image(&mut pool, alpha.id).id(), frame.id());

    for (asset, stored, rotation) in [
        (&exif, (64, 48), Rotation::Cw90),
        (&heic, (128, 64), Rotation::Cw270),
        (&webp, (64, 48), Rotation::None),
    ] {
        let video = asset.info.video.as_ref().unwrap();
        assert_eq!(video.rotation, rotation);
        let frame = exact_image(&mut pool, asset.id);
        let cpu = rgba(&frame);
        assert_eq!(cpu.visible, stored, "stored orientation");
        assert_eq!(upright(cpu.visible, video.rotation), video.display_size());
        assert_eq!(alpha_at(cpu, 5, 5), 255, "opaque sources");
    }
}

#[test]
fn images_are_capped_at_twice_the_canvas_long_side() {
    let heic = asset_at(&fixture("image-grid.heic"));
    let (mut pool, _) = pool(None, vec![heic.clone()]);
    let mut project = Project::new("tiny canvas");
    project.settings.width = 32;
    project.settings.height = 18;
    project.media = vec![heic.clone()];
    pool.set_project(Arc::new(project));
    let frame = exact_image(&mut pool, heic.id);
    assert_eq!(rgba(&frame).visible, (64, 32), "long side 2 x 32");
}
