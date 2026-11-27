//! Preview quality and zoom changes during playback (render-graph.md §8, task 0340): a new render
//! size re-converts the sources from the frames the readers already hold, so no reader seeks
//! again or reopens. Plays a clip forward frame by frame as the playing preview does
//! (`ReaderPool::set_rate(1)`, `BestAvailable`, the lookahead), changes the render size every
//! few frames through every quality and a zoom, and counts the readers' seeks and the readers.
//!
//! The default test runs on WARP with software decode on a committed fixture. The ignored one
//! runs on the hardware GPU with hardware decode on a clip outside the repository:
//!
//! ```text
//! CUTFRAME_PLAYBACK_MAIN=C:\clips\h264-1080p30.mp4 \
//!   cargo test --release -p engine --test preview_quality -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::media::MediaPath;
use app_core::{Clip, ClipSource, MediaAsset, MediaId, Project, Rgba8, Time};
use engine::media::readers::{ReaderOptions, ReaderPool};
use engine::media::{fingerprint, probe};
use engine::playback::scheduler::{self, Need};
use engine::playback::{Rate, Timeline};
use engine::render::{
    AdapterMode, Compositor, GpuContext, MissingLayers, PreviewQuality, PreviewView, PreviewZoom,
    build_plan, create_canvas, preview_render_size,
};

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

/// One main-track clip of the whole of `path`, at most `limit` long.
fn project_of(path: &Path, limit: Time) -> (Project, MediaId) {
    let asset = asset_at(path);
    let media = asset.id;
    let duration = asset.info.duration.expect("a video").min(limit);
    let mut project = Project::new("Quality");
    if let Some((width, height)) = asset.info.video.as_ref().map(|v| v.display_size()) {
        project.settings.width = width;
        project.settings.height = height;
    }
    let main = project.main_index().expect("main track");
    project.tracks[main].clips.push(Clip {
        source: ClipSource::media_at(media, Time::ZERO),
        ..Clip::color("V", Time::ZERO, duration, Rgba8::BLACK)
    });
    project.media.push(asset);
    (project, media)
}

/// What the preview cycles through: every quality at `Fit`, then 200 %.
const SETTINGS: [(PreviewQuality, PreviewView); 4] = [
    (PreviewQuality::Full, PreviewView::FIT),
    (PreviewQuality::Half, PreviewView::FIT),
    (PreviewQuality::Quarter, PreviewView::FIT),
    (
        PreviewQuality::Full,
        PreviewView {
            zoom: PreviewZoom::Percent(200),
            pan: (0.25, 0.0),
        },
    ),
];

/// Plays `project` frame by frame in a `window`-sized preview, switching [`SETTINGS`] every
/// `every` frames. Returns the seeks and readers after the first frame and at the end, and how
/// many render sizes were used.
fn play_switching(
    gpu: &Arc<GpuContext>,
    project: &Project,
    media: MediaId,
    window: (u32, u32),
    every: i64,
) -> ((u32, usize), (u32, usize), usize) {
    let mut pool = ReaderPool::new(Some(Arc::clone(gpu)), ReaderOptions::default(), |_| {});
    pool.set_project(project);
    pool.set_rate(1);
    let mut compositor = Compositor::new(gpu);
    let timeline = Timeline::of(project);
    let canvas_size = (project.settings.width, project.settings.height);
    let mut canvas = None;
    let mut sizes = Vec::new();
    let mut needs: Vec<Need> = Vec::new();
    let mut first = None;
    for frame in 0..=timeline.last_frame() {
        let (quality, view) = SETTINGS[(frame / every) as usize % SETTINGS.len()];
        let size = preview_render_size(canvas_size, window, quality, view);
        if canvas.as_ref().is_none_or(|(s, _, _)| *s != size) {
            let (texture, view) = create_canvas(&gpu.device, size);
            canvas = Some((size, texture, view));
            if !sizes.contains(&size) {
                sizes.push(size);
            }
        }
        let (_, _, canvas_view) = canvas.as_ref().expect("a canvas");
        let t = Time::from_frames(frame, timeline.frame_rate);
        let plan = build_plan(project, t, size);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let outcome = compositor.render_with(
                gpu,
                &mut encoder,
                &plan,
                &mut pool,
                canvas_view,
                MissingLayers::KeepCanvas,
            );
            gpu.queue.submit([encoder.finish()]);
            compositor.submitted(gpu);
            if outcome.stale_layers + outcome.missing_layers == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "frame {frame} never arrived");
            std::thread::sleep(Duration::from_millis(2));
        }
        compositor.release_completed();
        scheduler::lookahead(project, t, Rate::FORWARD, false, &mut needs);
        for need in &needs {
            pool.prime(need.media, need.sample_time, need.pace);
        }
        if frame == 0 {
            first = Some((pool.seek_count(media), pool.reader_count()));
        }
    }
    let last = (pool.seek_count(media), pool.reader_count());
    (first.expect("at least one frame"), last, sizes.len())
}

#[test]
fn switching_quality_and_zoom_while_playing_restarts_no_reader_on_warp() {
    let gpu = Arc::new(
        GpuContext::new_with(AdapterMode::SoftwareOnly)
            .expect("WARP (the D3D12 software adapter) ships with Windows"),
    );
    // 2 s of 30 fps with keyframes every 9 to 12 frames: decoding forward never needs a seek.
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media/numbered-gop.mp4");
    let (project, media) = project_of(&path, Time::from_seconds(2));
    // A 32 x 16 window: Fit and 200 % give different sizes for every quality.
    let (first, last, sizes) = play_switching(&gpu, &project, media, (32, 16), 3);
    assert_eq!(sizes, 4, "every setting composited at its own size");
    assert_eq!(
        first, last,
        "seeks and readers after the first frame stay the same"
    );
}

#[test]
#[ignore = "needs a GPU with video decode and CUTFRAME_PLAYBACK_MAIN"]
fn switching_quality_while_playing_restarts_no_hardware_reader() {
    let path = PathBuf::from(
        std::env::var("CUTFRAME_PLAYBACK_MAIN").expect("CUTFRAME_PLAYBACK_MAIN: a video clip"),
    );
    let gpu = Arc::new(GpuContext::new().expect("GPU"));
    eprintln!("adapter: {}", gpu.info().adapter);
    let (project, media) = project_of(&path, Time::from_seconds(10));
    let (first, last, sizes) = play_switching(&gpu, &project, media, (1280, 720), 15);
    eprintln!(
        "render sizes {sizes}; seeks/readers after the first frame {first:?}, at the end {last:?}"
    );
    assert_eq!(first, last, "a quality change seeks or opens a reader");
}
