//! Golden-frame tests of the render graph (render-graph.md §12, review F44).
//!
//! Every scene is a small project built on the committed media fixtures
//! (`tests/fixtures/media/`), rendered by `OfflineRenderer` (`FetchPolicy::Exact`,
//! `OutputFormat::Rgba8`) on WARP, the D3D12 software adapter: it ships with Windows, so every
//! machine and CI give the same pixels, and a missing adapter fails the tests instead of
//! skipping them. Each frame is compared with `tests/golden/<name>.png`: it passes when no
//! channel differs by more than 2/255 and at most 0.1 % of the pixels differ by more than
//! 1/255. On a mismatch the test writes `target/golden/<name>.actual.png` and
//! `<name>.diff.png` (differences x 32).
//!
//! ```text
//! CUTFRAME_UPDATE_GOLDEN=1 cargo test -p engine --test golden   # rewrite the references
//! ```
//!
//! Review the rewritten PNGs in the diff, like insta snapshots. Some scenes also assert
//! properties that do not depend on the references (exact colours, frame identities, the tone
//! map's look).
//!
//! The ignored test at the end runs on the hardware GPU (`--ignored`, local only).

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use app_core::media::MediaPath;
use app_core::{
    CanvasBackground, Clip, ClipSource, FrameRate, MediaAsset, MediaId, Project, Rgba8, Speed,
    Time, Track, TrackKind,
};
use engine::media::{fingerprint, probe};
use engine::render::{
    AdapterMode, FailedMedia, GpuContext, OfflineOptions, OfflineRenderer, OutputFormat,
    RenderError, RenderedFrame,
};

/// Largest per-channel difference of a passing frame.
const MAX_DIFF: u8 = 2;
/// Share of pixels allowed to differ by more than 1/255.
const MAX_OFF_BY_MORE_THAN_ONE: f64 = 0.001;

const UPDATE_ENV: &str = "CUTFRAME_UPDATE_GOLDEN";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> PathBuf {
    manifest().join("tests/fixtures/media").join(name)
}

/// WARP, shared by the tests of this binary.
fn warp() -> Arc<GpuContext> {
    static GPU: OnceLock<Arc<GpuContext>> = OnceLock::new();
    Arc::clone(GPU.get_or_init(|| {
        let gpu = GpuContext::new_with(AdapterMode::SoftwareOnly)
            .expect("WARP (the D3D12 software adapter) ships with Windows");
        let info = gpu.info();
        assert!(info.software, "not the software adapter: {info:?}");
        assert_eq!(info.backend, "Dx12", "{info:?}");
        Arc::new(gpu)
    }))
}

/// The GPU the scenes render on: WARP, unless the ignored hardware test replaced it.
fn scene_gpu() -> Arc<GpuContext> {
    let replaced = HARDWARE_GPU
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    replaced.unwrap_or_else(warp)
}

/// Set by [`hardware_decode_matches_software_and_every_scene_renders_without_it`] only.
static HARDWARE_GPU: Mutex<Option<Arc<GpuContext>>> = Mutex::new(None);

/// Golden scenes render one at a time: the tests share one device, and rendering them in
/// parallel on WARP only competes for the same CPU cores.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn asset(name: &str) -> MediaAsset {
    let path = fixture(name);
    let probed = probe(&path).unwrap_or_else(|err| panic!("{name}: {err}"));
    MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(&path).expect("fingerprint"),
        info: probed.info,
    }
}

fn ms(milliseconds: i64) -> Time {
    Time(milliseconds * app_core::FLICKS_PER_SECOND / 1000)
}

fn color_clip(start: Time, duration: Time, color: Rgba8) -> Clip {
    Clip::color("Colour", start, duration, color)
}

fn media_clip(asset: &MediaAsset, start: Time, duration: Time, source_in: Time) -> Clip {
    Clip {
        source: ClipSource::media_at(asset.id, source_in),
        ..Clip::color("Media", start, duration, Rgba8::BLACK)
    }
}

/// A project of `width` x `height` at 30 fps: `main` on the main track and `overlays` above it
/// (bottom to top), each `(clips, hidden)`.
fn project(
    (width, height): (u32, u32),
    media: Vec<MediaAsset>,
    main: Vec<Clip>,
    overlays: Vec<(Vec<Clip>, bool)>,
) -> Project {
    let mut project = Project::new("Golden");
    project.settings.width = width;
    project.settings.height = height;
    project.settings.frame_rate = FrameRate::FPS_30;
    project.background = CanvasBackground::Color {
        color: Rgba8::rgb(0x20, 0x40, 0x60),
    };
    project.media = media;
    project.tracks[0].clips = main;
    for (clips, hidden) in overlays {
        let mut track = Track::new(TrackKind::Video, "Overlay");
        track.clips = clips;
        track.hidden = hidden;
        project.tracks.insert(0, track);
    }
    project.validate().expect("a valid project");
    project
}

fn renderer(gpu: Arc<GpuContext>, failed_media: FailedMedia) -> OfflineRenderer {
    OfflineRenderer::new(
        gpu,
        OfflineOptions {
            failed_media,
            ..OfflineOptions::default()
        },
    )
}

/// Renders frame `n` of `project` at `size` as tight RGBA8.
fn render_with(
    renderer: &mut OfflineRenderer,
    project: &Project,
    n: i64,
    size: (u32, u32),
) -> Vec<u8> {
    match renderer.render_frame(project, n, size, OutputFormat::Rgba8) {
        Ok(RenderedFrame::Rgba8(rgba)) => rgba.to_vec(),
        Ok(RenderedFrame::Gpu(_)) => panic!("asked for Rgba8"),
        Err(err) => panic!("frame {n}: {err}"),
    }
}

/// Renders frame `n` of `project` at its canvas size on WARP.
fn render(project: &Project, n: i64) -> Vec<u8> {
    let size = (project.settings.width, project.settings.height);
    render_with(
        &mut renderer(scene_gpu(), FailedMedia::Placeholder),
        project,
        n,
        size,
    )
}

/// An RGBA8 picture.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picture {
    size: (u32, u32),
    rgba: Vec<u8>,
}

impl Picture {
    fn new(size: (u32, u32), rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), size.0 as usize * size.1 as usize * 4);
        Self { size, rgba }
    }

    fn pixels(&self) -> impl Iterator<Item = &[u8; 4]> {
        self.rgba.as_chunks::<4>().0.iter()
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.size.0 as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    fn pixels_in(
        &self,
        (x0, y0, x1, y1): (u32, u32, u32, u32),
    ) -> impl Iterator<Item = [u8; 4]> + '_ {
        (y0..y1).flat_map(move |y| (x0..x1).map(move |x| self.pixel(x, y)))
    }
}

fn write_png(path: &Path, picture: &Picture) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create the PNG folder");
    }
    let file = File::create(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let mut encoder = png::Encoder::new(BufWriter::new(file), picture.size.0, picture.size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().expect("PNG header");
    writer
        .write_image_data(&picture.rgba)
        .expect("PNG image data");
}

fn read_png(path: &Path) -> Picture {
    let file = File::open(path).unwrap_or_else(|err| {
        panic!(
            "{}: {err}. Create the reference with {UPDATE_ENV}=1 cargo test -p engine --test golden",
            path.display()
        )
    });
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .expect("PNG header");
    let mut rgba = vec![0; reader.output_buffer_size().expect("PNG size")];
    let info = reader.next_frame(&mut rgba).expect("PNG data");
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight),
        "{}",
        path.display()
    );
    rgba.truncate(info.buffer_size());
    Picture::new((info.width, info.height), rgba)
}

/// How far `actual` is from `reference`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Diff {
    max: u8,
    /// Where `max` is (the first such pixel).
    at: (u32, u32),
    /// Pixels with a channel that differs by more than 1/255.
    off_by_more_than_one: usize,
    /// Pixels with a channel that differs by more than 2/255.
    off_by_more_than_two: usize,
    pixels: usize,
}

impl Diff {
    fn of(reference: &Picture, actual: &Picture) -> Self {
        let mut max = 0;
        let mut at = 0;
        let mut off = 0;
        let mut off_two = 0;
        for (index, (a, b)) in reference.pixels().zip(actual.pixels()).enumerate() {
            let worst = a
                .iter()
                .zip(b)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            if worst > max {
                max = worst;
                at = index;
            }
            off += usize::from(worst > 1);
            off_two += usize::from(worst > 2);
        }
        let width = reference.size.0.max(1) as usize;
        Self {
            max,
            at: ((at % width) as u32, (at / width) as u32),
            off_by_more_than_one: off,
            off_by_more_than_two: off_two,
            pixels: reference.rgba.len() / 4,
        }
    }

    fn passes(&self) -> bool {
        self.max <= MAX_DIFF
            && self.off_by_more_than_one as f64 <= self.pixels as f64 * MAX_OFF_BY_MORE_THAN_ONE
    }
}

/// Compares `actual` with the reference `name`, or rewrites the reference when
/// [`UPDATE_ENV`] is `1`.
fn check(name: &str, actual: &Picture) {
    let reference_path = manifest().join("tests/golden").join(format!("{name}.png"));
    if std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1") {
        write_png(&reference_path, actual);
        eprintln!("updated {}", reference_path.display());
        return;
    }
    let reference = read_png(&reference_path);
    assert_eq!(reference.size, actual.size, "{name}: size");
    let diff = Diff::of(&reference, actual);
    if diff.passes() {
        return;
    }
    let out = manifest().join("../../target/golden");
    write_png(&out.join(format!("{name}.actual.png")), actual);
    let amplified = reference
        .pixels()
        .zip(actual.pixels())
        .flat_map(|(a, b)| {
            let d = |i: usize| a[i].abs_diff(b[i]).saturating_mul(32);
            [d(0), d(1), d(2), 255]
        })
        .collect();
    write_png(
        &out.join(format!("{name}.diff.png")),
        &Picture::new(actual.size, amplified),
    );
    panic!(
        "{name} differs from its reference: {diff:?} (allowed: max {MAX_DIFF}, at most {:.1} % \
         off by more than 1). See {}",
        MAX_OFF_BY_MORE_THAN_ONE * 100.0,
        out.display()
    );
}

/// Renders frame `n` at the canvas size and checks it against the reference `name`.
fn golden(name: &str, project: &Project, n: i64) -> Picture {
    let size = (project.settings.width, project.settings.height);
    let picture = Picture::new(size, render(project, n));
    check(name, &picture);
    picture
}

/// Rec.709 luma of an R'G'B' pixel, 0..=1.
fn luma([r, g, b, _]: [u8; 4]) -> f64 {
    (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
}

const SMALL: (u32, u32) = (64, 36);
const WIDE: (u32, u32) = (128, 72);
/// Where a square 64 x 64 video lands in a [`WIDE`] canvas with fit: 72 x 72 at x = 28.
const SQUARE_IN_WIDE: (u32, u32, u32, u32) = (28, 0, 100, 72);

#[test]
fn colour_background_and_colour_clips() {
    let _serial = serial();
    let project = project(
        SMALL,
        Vec::new(),
        vec![color_clip(Time::ZERO, ms(1000), Rgba8::rgb(200, 40, 10))],
        vec![(
            vec![Clip {
                visual: Some(app_core::props::VisualProps {
                    opacity: app_core::props::Animated::constant(0.5),
                    ..Default::default()
                }),
                ..color_clip(ms(1000), ms(1000), Rgba8::rgb(255, 255, 255))
            }],
            false,
        )],
    );
    let clip = golden("colour_clip", &project, 0);
    assert!(
        clip.pixels_in((0, 0, 64, 36))
            .all(|p| p == [200, 40, 10, 255])
    );
    let background = golden("colour_background", &project, 75);
    assert!(
        background
            .pixels_in((0, 0, 64, 36))
            .all(|p| p == [0x20, 0x40, 0x60, 255])
    );
    // Half-transparent white over the background: (white + background) / 2.
    let half = golden("colour_half_opacity", &project, 45);
    let expected = [143.5, 159.5, 175.5];
    for p in half.pixels_in((0, 0, 64, 36)) {
        for c in 0..3 {
            assert!((f64::from(p[c]) - expected[c]).abs() <= 1.0, "{p:?}");
        }
    }
}

fn sdr_project() -> Project {
    let video = asset("h264-aac.mp4");
    let main = vec![media_clip(&video, Time::ZERO, ms(1000), Time::ZERO)];
    project(WIDE, vec![video], main, Vec::new())
}

#[test]
fn sdr_h264_frames_by_pts() {
    let _serial = serial();
    let project = sdr_project();
    let frames: Vec<Picture> = [0, 10, 29]
        .into_iter()
        .map(|n| golden(&format!("sdr_h264_f{n:02}"), &project, n))
        .collect();
    // Three different source frames; the pillarbox is the background.
    assert_ne!(frames[0], frames[1]);
    assert_ne!(frames[1], frames[2]);
    for frame in &frames {
        assert_eq!(frame.pixel(2, 36), [0x20, 0x40, 0x60, 255]);
        assert_eq!(frame.pixel(125, 36), [0x20, 0x40, 0x60, 255]);
    }
}

#[test]
fn sdr_h264_at_half_render_size() {
    let _serial = serial();
    let project = sdr_project();
    let rgba = render_with(
        &mut renderer(scene_gpu(), FailedMedia::Placeholder),
        &project,
        10,
        (64, 36),
    );
    check("sdr_h264_half", &Picture::new((64, 36), rgba));
}

#[test]
fn portrait_hevc_is_upright() {
    let _serial = serial();
    let video = asset("hevc-portrait-cw90-vfr.mov");
    let main = vec![media_clip(&video, Time::ZERO, ms(2000), Time::ZERO)];
    let project = project(WIDE, vec![video], main, Vec::new());
    golden("portrait_hevc_cw90", &project, 15);
}

#[test]
fn anamorphic_mpeg2_has_its_display_shape() {
    let _serial = serial();
    let video = asset("mpeg2-anamorphic.mpg");
    let main = vec![media_clip(&video, Time::ZERO, ms(300), Time::ZERO)];
    // 144 x 108 with 4:3 pixels is 192 x 108 on screen: it fills a 192 x 108 canvas.
    let project = project((192, 108), vec![video], main, Vec::new());
    let picture = golden("anamorphic_mpeg2", &project, 5);
    let background = [0x20, 0x40, 0x60, 255];
    assert!(
        picture.pixels_in((0, 0, 1, 108)).all(|p| p != background)
            && picture
                .pixels_in((191, 0, 192, 108))
                .all(|p| p != background),
        "no pillarbox: the picture is 192 pixels wide"
    );
}

#[test]
fn png_with_alpha_over_a_colour_clip() {
    let _serial = serial();
    let image = asset("image-alpha.png");
    let colour = Rgba8::rgb(20, 120, 200);
    let project = project(
        (128, 96),
        vec![image.clone()],
        vec![color_clip(Time::ZERO, ms(1000), colour)],
        vec![(
            vec![media_clip(&image, Time::ZERO, ms(1000), Time::ZERO)],
            false,
        )],
    );
    let picture = golden("image_alpha_over_colour", &project, 0);
    // Alpha 0 in the left column: the colour clip shows through.
    for y in 0..96 {
        let p = picture.pixel(0, y);
        assert!(
            p[..3]
                .iter()
                .zip([colour.r, colour.g, colour.b])
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "{p:?}"
        );
    }
}

#[test]
fn jpeg_with_exif_orientation_is_upright() {
    let _serial = serial();
    let image = asset("exif-orientation-6.jpg");
    let main = vec![media_clip(&image, Time::ZERO, ms(1000), Time::ZERO)];
    let project = project(WIDE, vec![image], main, Vec::new());
    let picture = golden("jpeg_exif_orientation_6", &project, 0);
    // 48 x 64 upright: 54 x 72 in the middle, the background left and right of it.
    assert_eq!(picture.pixel(30, 36), [0x20, 0x40, 0x60, 255]);
    assert_ne!(picture.pixel(64, 36), [0x20, 0x40, 0x60, 255]);
}

/// The tone map is wired (render-graph.md §5.2): the HDR picture is neither washed out to grey
/// nor clipped, and its mid-tones are close to the SDR version of the same test pattern.
fn assert_natural_hdr(name: &str, hdr: &Picture, sdr: &Picture) {
    let pixels: Vec<[u8; 4]> = hdr.pixels_in(SQUARE_IN_WIDE).collect();
    let mean = pixels.iter().copied().map(luma).sum::<f64>() / pixels.len() as f64;
    let sdr_mean = sdr.pixels_in(SQUARE_IN_WIDE).map(luma).sum::<f64>() / pixels.len() as f64;
    let saturated = pixels
        .iter()
        .filter(|p| {
            let max = p[..3].iter().max().copied().unwrap_or(0);
            let min = p[..3].iter().min().copied().unwrap_or(0);
            max - min >= 100
        })
        .count() as f64
        / pixels.len() as f64;
    let clipped = pixels
        .iter()
        .filter(|p| p[..3].iter().all(|&c| c >= 254))
        .count() as f64
        / pixels.len() as f64;
    eprintln!(
        "{name}: mean luma {mean:.3} (SDR {sdr_mean:.3}), saturated {:.1} %, clipped white {:.1} %",
        saturated * 100.0,
        clipped * 100.0
    );
    assert!(
        (0.2..=0.8).contains(&mean) && (mean - sdr_mean).abs() < 0.2,
        "{name}: mean luma {mean:.3} against SDR {sdr_mean:.3}"
    );
    assert!(
        saturated > 0.2,
        "{name}: grey-washed ({saturated:.3} saturated)"
    );
    assert!(clipped < 0.1, "{name}: clipped ({clipped:.3} white)");
}

#[test]
fn hdr_hlg_and_pq_are_tone_mapped() {
    let _serial = serial();
    // The HDR fixtures are 0.3 s long: frame 5 of each, against frame 5 of the SDR fixture
    // (the same test pattern).
    let sdr = Picture::new(WIDE, render(&sdr_project(), 5));
    for (name, file) in [
        ("hdr_hlg_hevc", "hevc-hlg-dv84.mov"),
        ("hdr_pq_av1_10bit", "av1-10bit-pq.mp4"),
    ] {
        let video = asset(file);
        let main = vec![media_clip(&video, Time::ZERO, ms(300), Time::ZERO)];
        let project = project(WIDE, vec![video], main, Vec::new());
        let picture = golden(name, &project, 5);
        assert_natural_hdr(name, &picture, &sdr);
    }
}

#[test]
fn overlays_draw_above_main_and_hidden_tracks_not_at_all() {
    let _serial = serial();
    let video = asset("h264-aac.mp4");
    let image = asset("image-alpha.png");
    let hidden = Rgba8::rgb(255, 0, 255);
    let project = project(
        WIDE,
        vec![video.clone(), image.clone()],
        vec![media_clip(&video, Time::ZERO, ms(1000), Time::ZERO)],
        vec![
            (
                vec![media_clip(&image, Time::ZERO, ms(1000), Time::ZERO)],
                false,
            ),
            (vec![color_clip(Time::ZERO, ms(1000), hidden)], true),
        ],
    );
    let picture = golden("overlay_order", &project, 10);
    assert!(
        picture
            .pixels_in((0, 0, 128, 72))
            .all(|p| p != [255, 0, 255, 255])
    );
    // The 4:3 image (96 x 72 at x = 16) is drawn over the video's pillarbox (x >= 100).
    let under = Picture::new(WIDE, render(&sdr_project(), 10));
    assert_eq!(under.pixel(108, 36), [0x20, 0x40, 0x60, 255]);
    assert_ne!(picture.pixel(108, 36), under.pixel(108, 36));
}

#[test]
fn a_cut_point_shows_the_next_clip() {
    let _serial = serial();
    let video = asset("h264-aac.mp4");
    // Two clips of the same file, both from its start: at 0.5 s the second clip's first frame
    // shows, not frame 15 of the first clip.
    let main = vec![
        media_clip(&video, Time::ZERO, ms(500), Time::ZERO),
        media_clip(&video, ms(500), ms(500), Time::ZERO),
    ];
    let project = project(WIDE, vec![video], main, Vec::new());
    let cut = golden("cut_point", &project, 15);
    let first = Picture::new(WIDE, render(&sdr_project(), 0));
    assert_eq!(cut, first, "the next clip's first frame");
}

// ---- Speed, reverse and freeze frames (`clip-speed-audio.md` §9) ----------------------------

/// The canvas of the numbered fixture: 64 x 32, the picture fills it.
const NUMBERED: (u32, u32) = (64, 32);

/// The source frame a render of `numbered-gop.mp4` shows: bit b is the bar at x = 8b..8b+7.
fn shown_frame(picture: &Picture) -> u32 {
    (0..8)
        .filter(|&bit| luma(picture.pixel(8 * bit + 4, 16)) > 0.5)
        .map(|bit| 1 << bit)
        .sum()
}

/// A media clip of `asset` at `percent` speed, `reverse`d or not.
fn paced_clip(
    asset: &MediaAsset,
    start: Time,
    duration: Time,
    source_in: Time,
    percent: u16,
    reverse: bool,
) -> Clip {
    Clip {
        source: ClipSource::Media {
            media: asset.id,
            source_in,
            speed: Speed::new(percent).expect("speed"),
            reverse,
        },
        ..media_clip(asset, start, duration, source_in)
    }
}

fn numbered_project(main: Vec<Clip>, video: MediaAsset) -> Project {
    project(NUMBERED, vec![video], main, Vec::new())
}

#[test]
fn speed_2x_and_half_speed_show_the_mapped_source_frames() {
    let _serial = serial();
    let video = asset("numbered-gop.mp4");
    // 2x over source 0..2 s: project frame n covers source frames 2n and 2n + 1 and shows the
    // one at its centre, 2n + 1.
    let fast = numbered_project(
        vec![paced_clip(
            &video,
            Time::ZERO,
            ms(1000),
            Time::ZERO,
            200,
            false,
        )],
        video.clone(),
    );
    let picture = golden("speed_2x", &fast, 10);
    assert_eq!(shown_frame(&picture), 21);
    let mut offline = renderer(scene_gpu(), FailedMedia::Error);
    for n in [0, 5, 12, 29] {
        let picture = Picture::new(NUMBERED, render_with(&mut offline, &fast, n, NUMBERED));
        assert_eq!(shown_frame(&picture), 2 * n as u32 + 1, "2x frame {n}");
    }
    // 0.5x from source 0.5 s: every source frame for two project frames.
    let slow = numbered_project(
        vec![paced_clip(&video, Time::ZERO, ms(2000), ms(500), 50, false)],
        video,
    );
    let picture = golden("speed_half", &slow, 7);
    assert_eq!(shown_frame(&picture), 18);
    let mut offline = renderer(scene_gpu(), FailedMedia::Error);
    let shown: Vec<u32> = (0..12)
        .map(|n| {
            shown_frame(&Picture::new(
                NUMBERED,
                render_with(&mut offline, &slow, n, NUMBERED),
            ))
        })
        .collect();
    assert_eq!(shown, [15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20]);
}

/// Export renders a reversed clip exactly: every frame equals the forward render of the source
/// frame it maps to, across every GOP boundary (keyframes at 0, 12, 21, 30, 39, 48, 57).
#[test]
fn a_reversed_clip_plays_the_source_backwards_exactly() {
    let _serial = serial();
    let video = asset("numbered-gop.mp4");
    let reversed = numbered_project(
        vec![paced_clip(
            &video,
            Time::ZERO,
            ms(2000),
            Time::ZERO,
            100,
            true,
        )],
        video.clone(),
    );
    let forward = numbered_project(
        vec![media_clip(&video, Time::ZERO, ms(2000), Time::ZERO)],
        video,
    );
    let first = golden("reverse_f00", &reversed, 0);
    assert_eq!(shown_frame(&first), 59, "the end of the range first");
    let across = golden("reverse_f03", &reversed, 3);
    assert_eq!(shown_frame(&across), 56, "below the keyframe at 57");
    let mut backwards = renderer(scene_gpu(), FailedMedia::Error);
    let mut ahead = renderer(scene_gpu(), FailedMedia::Error);
    for n in 0..60 {
        let back = render_with(&mut backwards, &reversed, n, NUMBERED);
        let fwd = render_with(&mut ahead, &forward, 59 - n, NUMBERED);
        assert!(
            back == fwd,
            "reversed frame {n} differs from forward frame {}",
            59 - n
        );
        assert_eq!(shown_frame(&Picture::new(NUMBERED, back)), 59 - n as u32);
    }
}

/// A freeze frame inserted at the playhead shows the frame the clip showed there, held.
#[test]
fn a_freeze_frame_holds_the_frame_under_the_playhead() {
    let _serial = serial();
    let video = asset("numbered-gop.mp4");
    let frame = FrameRate::FPS_30.frame_duration();
    let original = media_clip(&video, Time::ZERO, ms(2000), Time::ZERO);
    // The playhead at project frame 20 (InsertFreezeFrame samples like the render plan).
    let at = Time(20 * frame.flicks());
    let source_time = original
        .frame_sample_time(at, frame)
        .expect("inside the clip");
    let before = Picture::new(
        NUMBERED,
        render(&numbered_project(vec![original], video.clone()), 20),
    );
    let left = media_clip(&video, Time::ZERO, at, Time::ZERO);
    let freeze = Clip {
        source: ClipSource::Freeze {
            media: video.id,
            source_time,
        },
        ..Clip::color("Freeze", at, ms(1000), Rgba8::BLACK)
    };
    let right = media_clip(&video, at + ms(1000), ms(1000), at);
    let project = numbered_project(vec![left, freeze, right], video);
    let held = golden("freeze_frame", &project, 20);
    assert_eq!(held, before, "the frame the clip showed");
    assert_eq!(shown_frame(&held), 20);
    let mut offline = renderer(scene_gpu(), FailedMedia::Error);
    for n in [19, 20, 35, 49, 50] {
        let picture = Picture::new(NUMBERED, render_with(&mut offline, &project, n, NUMBERED));
        let expected = match n {
            ..20 => n,
            20..50 => 20,
            _ => n - 30,
        };
        assert_eq!(shown_frame(&picture), expected as u32, "frame {n}");
    }
}

/// A copy of `asset` whose file is `path`.
fn moved(asset: &MediaAsset, path: &Path) -> MediaAsset {
    MediaAsset {
        id: MediaId::new(),
        path: MediaPath::absolute(path.to_string_lossy()),
        ..asset.clone()
    }
}

#[test]
fn offline_and_undecodable_media_show_placeholders() {
    let _serial = serial();
    let video = asset("h264-aac.mp4");
    let offline = moved(&video, &fixture("does-not-exist.mp4"));
    let broken = moved(&video, &fixture("random-bytes.mp4"));
    // The placeholder covers the layer's rectangle: the 64 x 64 video fitted into 64 x 36.
    let colours = |picture: &Picture| {
        let mut colours: Vec<[u8; 4]> = picture.pixels_in((14, 0, 50, 36)).collect();
        colours.sort_unstable();
        colours.dedup();
        colours
    };
    for (name, asset, error) in [
        (
            "placeholder_offline",
            &offline,
            RenderError::MediaOffline(offline.id),
        ),
        (
            "placeholder_decode_error",
            &broken,
            RenderError::Decode(broken.id),
        ),
    ] {
        let main = vec![media_clip(asset, Time::ZERO, ms(1000), Time::ZERO)];
        let project = project(SMALL, vec![asset.clone()], main, Vec::new());
        let picture = golden(name, &project, 0);
        assert_eq!(colours(&picture).len(), 2, "{name}: two stripe colours");
        // Export fails instead (render-graph.md §4.5).
        let mut strict = renderer(scene_gpu(), FailedMedia::Error);
        match strict.render_frame(&project, 0, SMALL, OutputFormat::Rgba8) {
            Err(err) => assert_eq!(err, error, "{name}"),
            Ok(_) => panic!("{name}: rendered without an error"),
        }
    }
    let offline_main = vec![media_clip(&offline, Time::ZERO, ms(1000), Time::ZERO)];
    let broken_main = vec![media_clip(&broken, Time::ZERO, ms(1000), Time::ZERO)];
    let a = render(
        &project(SMALL, vec![offline.clone()], offline_main, Vec::new()),
        0,
    );
    let b = render(
        &project(SMALL, vec![broken.clone()], broken_main, Vec::new()),
        0,
    );
    assert_ne!(
        colours(&Picture::new(SMALL, a)),
        colours(&Picture::new(SMALL, b)),
        "offline and decode error use different colours"
    );
}

/// Consecutive frames reuse the canvas and read-back targets (review F48) and give the same
/// pixels as fresh renderers.
#[test]
fn one_renderer_renders_a_sequence() {
    let _serial = serial();
    let project = sdr_project();
    let mut sequence = renderer(scene_gpu(), FailedMedia::Placeholder);
    for n in [0, 1, 2, 10, 10, 29] {
        let reused = render_with(&mut sequence, &project, n, WIDE);
        assert_eq!(reused, render(&project, n), "frame {n}");
    }
}

/// Hardware decode (render-graph.md §12 "Hardware tests"), local only, on the hardware GPU
/// (`CUTFRAME_GPU=integrated|discrete` picks it):
///
/// 1. Every clip in `CUTFRAME_HW_CLIPS` (paths separated by `;`, e.g. 4K HEVC Main 10 HLG and
///    4K H.264 phone clips) renders through the zero-copy path like the same frames decoded in
///    software (`CUTFRAME_HWDECODE=off`), at the canvas size and at 1920 x 1080: no pixel more
///    than 8/255 off and at most 1 in 100,000 more than 2/255 (see the comment at the check).
/// 2. With `CUTFRAME_HWDECODE=off`, every golden scene renders on the hardware GPU; each is
///    compared with its WARP reference and the result is printed (filtering on a hardware GPU
///    may differ from WARP in the last bit).
///
/// ```text
/// CUTFRAME_HW_CLIPS="C:\clips\hevc.mp4;C:\clips\h264.mp4" \
///   cargo test -p engine --release --test golden -- --ignored --nocapture
/// ```
///
/// Run it alone (`--ignored`, not `--include-ignored`): it sets `CUTFRAME_HWDECODE` while it
/// runs.
#[test]
#[ignore = "needs a GPU with D3D12 video decode; run locally with --ignored"]
fn hardware_decode_matches_software_and_every_scene_renders_without_it() {
    let gpu = Arc::new(GpuContext::new().expect("GPU"));
    let info = gpu.info();
    eprintln!("adapter: {} ({:?})", info.adapter, info.device_type);
    assert!(!info.software, "needs a hardware adapter");

    let clips: Vec<PathBuf> = std::env::var("CUTFRAME_HW_CLIPS")
        .map(|list| {
            list.split(';')
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    if clips.is_empty() {
        eprintln!("CUTFRAME_HW_CLIPS is not set: skipping the zero-copy comparison");
    }
    let frames = [0, 1, 15, 45, 90];
    // (clip, project, size, hardware frames, decode path)
    let mut zero_copy = Vec::new();
    for clip in &clips {
        let video = {
            let probed = probe(clip).unwrap_or_else(|err| panic!("{}: {err}", clip.display()));
            MediaAsset {
                id: MediaId::new(),
                kind: probed.kind,
                path: MediaPath::absolute(clip.to_string_lossy()),
                fingerprint: fingerprint(clip).expect("fingerprint"),
                info: probed.info,
            }
        };
        let display = video.info.video.as_ref().expect("a video").display_size();
        let duration = video.info.duration.expect("a duration");
        let main = vec![media_clip(&video, Time::ZERO, duration, Time::ZERO)];
        let project = project(display, vec![video.clone()], main, Vec::new());
        // The canvas size (1:1, the usual export) and 1080p (a 2x downscale with 2 x 2 taps).
        for size in [display, (1920, 1080)] {
            let mut hw = renderer(Arc::clone(&gpu), FailedMedia::Error);
            let rendered: Vec<Vec<u8>> = frames
                .iter()
                .map(|&n| render_with(&mut hw, &project, n, size))
                .collect();
            let path = hw.decode_path(video.id);
            zero_copy.push((clip.clone(), project.clone(), size, rendered, path));
        }
    }

    // SAFETY: this ignored test runs alone (see the doc comment): no other thread reads the
    // environment meanwhile; the hardware renderers above are idle.
    unsafe { std::env::set_var("CUTFRAME_HWDECODE", "off") };
    let mut mismatches = Vec::new();
    for (clip, project, size, rendered, path) in zero_copy {
        let mut sw = renderer(Arc::clone(&gpu), FailedMedia::Error);
        for (&n, hw_rgba) in frames.iter().zip(&rendered) {
            let sw_picture = Picture::new(size, render_with(&mut sw, &project, n, size));
            let hw_picture = Picture::new(size, hw_rgba.clone());
            let diff = Diff::of(&sw_picture, &hw_picture);
            eprintln!(
                "{} {}x{} frame {n}: {path:?} against software: {diff:?}",
                clip.display(),
                size.0,
                size.1
            );
            // A few pixels whose darkest channel sits at the gamut clip after the HDR tone map
            // may differ by a little more: the BT.1886 encode (L^(1/2.4)) has an infinite slope
            // at 0 and turns f32 rounding (P010 against LSB-aligned 10-bit planes) into a few
            // code values. Allowed: none above 8/255, at most 1 in 100,000 above 2/255.
            let ok = diff.max <= 8 && diff.off_by_more_than_two * 100_000 <= diff.pixels;
            if !ok {
                let stem = clip.file_stem().unwrap_or_default().to_string_lossy();
                let out = manifest().join("../../target/golden");
                write_png(&out.join(format!("{stem}_{n}.hardware.png")), &hw_picture);
                write_png(&out.join(format!("{stem}_{n}.software.png")), &sw_picture);
                mismatches.push(format!("{} {size:?} frame {n}: {diff:?}", clip.display()));
            }
        }
        assert_eq!(
            path,
            Some(engine::media::readers::DecodePath::ZeroCopy),
            "{}",
            clip.display()
        );
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");

    *HARDWARE_GPU
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&gpu));
    let scenes: [(&str, fn()); 14] = [
        ("colour", colour_background_and_colour_clips),
        ("sdr frames", sdr_h264_frames_by_pts),
        ("sdr half", sdr_h264_at_half_render_size),
        ("portrait", portrait_hevc_is_upright),
        ("anamorphic", anamorphic_mpeg2_has_its_display_shape),
        ("png alpha", png_with_alpha_over_a_colour_clip),
        ("jpeg exif", jpeg_with_exif_orientation_is_upright),
        ("hdr", hdr_hlg_and_pq_are_tone_mapped),
        (
            "overlays",
            overlays_draw_above_main_and_hidden_tracks_not_at_all,
        ),
        ("cut point", a_cut_point_shows_the_next_clip),
        (
            "placeholders",
            offline_and_undecodable_media_show_placeholders,
        ),
        (
            "speed",
            speed_2x_and_half_speed_show_the_mapped_source_frames,
        ),
        (
            "reverse",
            a_reversed_clip_plays_the_source_backwards_exactly,
        ),
        ("freeze", a_freeze_frame_holds_the_frame_under_the_playhead),
    ];
    let mut failed = Vec::new();
    for (name, scene) in scenes {
        match std::panic::catch_unwind(scene) {
            Ok(()) => eprintln!("software decode on {}: {name} matches WARP", info.adapter),
            Err(_) => failed.push(name),
        }
    }
    *HARDWARE_GPU
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    assert!(
        failed.is_empty(),
        "scenes that differ from WARP on {}: {failed:?} (see target/golden)",
        info.adapter
    );
}
