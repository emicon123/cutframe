//! Hardware decode through the reader pool (render-graph.md §5.4). Ignored by default: it needs a
//! GPU with D3D12 video decode and a large clip that is not in the repository.
//!
//! ```text
//! CUTFRAME_GPU=discrete CUTFRAME_HW_CLIP=C:\clips\hevc-4k60-main10-hlg.mp4 \
//!   cargo test -p engine --test readers_hardware -- --ignored --nocapture
//! ```
//!
//! This binary holds one test only: it sets `CUTFRAME_HWDECODE` for its second half, and no
//! other test may run (and read the environment) meanwhile.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_core::media::MediaPath;
use app_core::{FLICKS_PER_SECOND, MediaAsset, MediaId, Project, Time};
use engine::media::readers::{DecodePath, ReaderOptions, ReaderPool};
use engine::media::{FrameData, fingerprint, probe};
use engine::render::{FetchPolicy, FrameLookup, FrameProvider, FrameRef, GpuContext};

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

fn pool(gpu: Option<Arc<GpuContext>>, asset: &MediaAsset) -> ReaderPool {
    let mut pool = ReaderPool::new(gpu, ReaderOptions::default(), |_| {});
    let mut project = Project::new("hardware");
    project.media = vec![asset.clone()];
    pool.set_project(Arc::new(project));
    pool
}

fn exact(pool: &mut ReaderPool, media: MediaId, t: Time) -> FrameRef {
    pool.begin_frame();
    match pool.video(media, t, FetchPolicy::Exact) {
        FrameLookup::Exact(frame) => frame,
        other => panic!("expected an exact frame at {t:?}, got {other:?}"),
    }
}

/// What the compositor does with a zero-copy frame: sample it, record the release, submit, and
/// drop the frame only after the submission completed.
fn sample_and_release(gpu: &GpuContext, frame: FrameRef) {
    let FrameData::Gpu(surface) = frame.data() else {
        panic!("not a GPU frame");
    };
    let views = surface.plane_views();
    assert_eq!(views.len(), 2);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    surface.record_release(&mut encoder);
    let index = gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })
        .expect("poll");
    drop(views);
    drop(frame);
}

#[test]
#[ignore = "needs hardware video decode and CUTFRAME_HW_CLIP"]
fn hardware_frames_are_zero_copy_and_match_software_identities() {
    let gpu = Arc::new(GpuContext::new().expect("GPU"));
    eprintln!("adapter: {:?}", gpu.info());
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media");

    match std::env::var_os("CUTFRAME_HW_CLIP") {
        None => eprintln!("CUTFRAME_HW_CLIP is not set: skipping the zero-copy half"),
        Some(clip) => {
            let asset = asset_at(Path::new(&clip));
            let mut hw = pool(Some(Arc::clone(&gpu)), &asset);
            let mut sw = pool(None, &asset);
            let second = FLICKS_PER_SECOND;
            for t in [0, second / 3, 2 * second, second / 2, 5 * second] {
                let t = Time::from_flicks(t);
                let started = std::time::Instant::now();
                let frame = exact(&mut hw, asset.id, t);
                let hw_ms = started.elapsed().as_millis();
                assert!(
                    matches!(frame.data(), FrameData::Gpu(_)),
                    "zero-copy at {t:?}: {:?}",
                    hw.decode_path(asset.id)
                );
                assert_eq!(hw.decode_path(asset.id), Some(DecodePath::ZeroCopy));
                let reference = exact(&mut sw, asset.id, t);
                assert!(reference.cpu().is_some());
                assert_eq!(frame.id(), reference.id(), "same frame identity at {t:?}");
                eprintln!(
                    "t = {:.3} s: pts {:?}, {:?} visible {:?}, hardware {hw_ms} ms",
                    t.as_seconds_f64(),
                    frame.pts(),
                    frame.gpu().map(|g| g.coded),
                    frame.visible(),
                );
                sample_and_release(&gpu, frame);
            }
            // Playback: the queue fills and every frame comes back in order.
            hw.set_playing(true);
            let step = second / 60;
            let mut last = None;
            for n in 0..120 {
                let frame = exact(&mut hw, asset.id, Time::from_flicks(n * step));
                assert!(frame.pts() >= last);
                last = frame.pts();
                sample_and_release(&gpu, frame);
            }
        }
    }

    // CUTFRAME_HWDECODE=off: every committed video fixture decodes in software.
    // SAFETY: the only test in this binary sets the variable before any reader of it runs; the
    // pools above are idle (their readers wait on their own condition variables).
    unsafe { std::env::set_var("CUTFRAME_HWDECODE", "off") };
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
        let asset = asset_at(&fixtures.join(name));
        let mut pool = pool(Some(Arc::clone(&gpu)), &asset);
        let frame = exact(
            &mut pool,
            asset.id,
            Time::from_flicks(FLICKS_PER_SECOND / 3),
        );
        assert!(frame.cpu().is_some(), "{name}");
        assert_eq!(
            pool.decode_path(asset.id),
            Some(DecodePath::Software),
            "{name}"
        );
    }
    if let Some(clip) = std::env::var_os("CUTFRAME_HW_CLIP") {
        let asset = asset_at(Path::new(&clip));
        let mut pool = pool(Some(Arc::clone(&gpu)), &asset);
        let frame = exact(&mut pool, asset.id, Time::from_flicks(FLICKS_PER_SECOND));
        assert!(frame.cpu().is_some(), "forced software");
        assert_eq!(pool.decode_path(asset.id), Some(DecodePath::Software));
    }
}
