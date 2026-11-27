//! The pure half of the render graph (design `docs/design/render-graph.md` §4.1, §4.2): a
//! [`RenderPlan`] says what one project frame contains, built from a snapshot by [`build_plan`]
//! without a GPU or FFmpeg. The compositor executes it; preview, export, `--smoke-test` and the
//! golden tests share it, and it is compared with the previous plan (`PartialEq`) to skip
//! redundant work.
//!
//! What task 0320 fills in: the frame grid, layer order, layer content, the fit rectangle and the
//! opacity. Crop, scale, rotation, position, flip and the `Blur` / `Image` canvas backgrounds are
//! task 0350; until then the plan carries zero crop and a fit-only transform, and those
//! backgrounds fall back to black (see [`BackgroundPlan`]).

use app_core::media::MediaKind;
use app_core::props::CropInsets;
use app_core::{
    CanvasBackground, Clip, ClipId, ClipSource, FitMode, MediaId, Project, Rgba8, Time, Track,
    TrackKind,
};

use super::convert::ConvertParams;
use super::frames::PlaceholderKind;

/// Everything needed to draw one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    /// The project frame this plan draws: `t` quantised down to the project frame grid.
    pub frame: i64,
    /// Start of [`Self::frame`]: the time clips and properties are evaluated at.
    pub time: Time,
    /// Canvas size in project pixels (`ProjectSettings` width x height).
    pub canvas: (u32, u32),
    /// Size of the composited texture in pixels (§8), as passed to [`build_plan`].
    pub render_size: (u32, u32),
    /// What fills the canvas before the layers.
    pub background: BackgroundPlan,
    /// Bottom to top.
    pub layers: Vec<LayerPlan>,
}

/// The canvas background of a plan.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundPlan {
    /// Clear to this colour (taken as R'G'B', straight alpha).
    ///
    /// TODO(0350): `CanvasBackground::Blur` and `CanvasBackground::Image` get their own variants
    /// (a blurred fill of the main-track layer, an image drawn with fill). Until then both are
    /// planned as opaque black, the default background.
    Color(Rgba8),
}

/// One clip drawn over the layers below it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlan {
    pub clip: ClipId,
    pub content: LayerContent,
    /// Placement of the layer's unit quad in canvas pixels. In 0320 only the fit rectangle
    /// (`VisualProps::fit`); 0350 adds crop, scale, rotation, position and flip.
    pub transform: Affine2,
    /// Fractions of the upright source cut from each side. Always zero in 0320 (0350).
    pub crop: CropInsets,
    /// 0..=1, `VisualProps::opacity` at the plan's time. Applied to the premultiplied layer.
    pub opacity: f32,
}

/// What a layer draws.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerContent {
    /// A video frame: the last decoded frame with `pts <= sample_time` (sample and hold).
    Video {
        media: MediaId,
        /// Media time at the centre of the project frame (§4.1 "Time and sampling").
        sample_time: Time,
        convert: ConvertParams,
    },
    /// A still image.
    Image {
        media: MediaId,
        convert: ConvertParams,
    },
    /// A solid colour card (straight alpha) over the whole layer rectangle.
    Color(Rgba8),
    /// Media that cannot be drawn: offline or unreadable (§4.3, §10).
    Placeholder(PlaceholderKind),
}

/// A 2x3 affine matrix from the unit quad (`u`, `v` in `0..=1`, `(0, 0)` top left, `v` down) to
/// canvas pixels: `x = rows[0][0] u + rows[0][1] v + rows[0][2]`, and `y` likewise from `rows[1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine2 {
    pub rows: [[f32; 3]; 2],
}

impl Affine2 {
    /// Maps the unit quad onto the axis-aligned rectangle at `(x, y)` with size `(w, h)`.
    pub const fn rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            rows: [[w, 0.0, x], [0.0, h, y]],
        }
    }

    /// The canvas point of unit-quad point `(u, v)`.
    pub fn apply(&self, (u, v): (f32, f32)) -> (f32, f32) {
        let [x, y] = self.rows;
        (x[0] * u + x[1] * v + x[2], y[0] * u + y[1] * v + y[2])
    }
}

/// Builds the plan of the project frame that contains `t` (render-graph.md §4.1, §4.2).
///
/// - `t` is quantised down to frame `n` of the project rate; clips and properties are evaluated
///   at the frame start `t_n`. A clip is active when `start <= t_n < end`.
/// - Layers, bottom to top: the main track, then the overlay tracks from the bottom of the
///   overlay section to the top (timeline-editing.md §3.3, §7). Hidden tracks, audio tracks and
///   text tracks (0510) draw nothing.
/// - Video layers sample the source at the centre of the project frame:
///   `sample_time = source_time(t_n) + frame_duration / 2`. Images have no time.
/// - A media id that is not in `project.media` gives an `Offline` placeholder over the canvas.
///   Media without a video stream (audio-only media, which `Project::validate` keeps off video
///   tracks) draws nothing.
pub fn build_plan(project: &Project, t: Time, render_size: (u32, u32)) -> RenderPlan {
    let rate = project.settings.frame_rate;
    let frame = t.to_frames(rate);
    let time = Time::from_frames(frame, rate);
    let half_frame = Time(rate.frame_duration().flicks() / 2);
    let canvas = (project.settings.width, project.settings.height);
    // The main track and everything above it; without a main track (a broken project) all tracks.
    let drawn = project
        .main_index()
        .map_or(project.tracks.len(), |main| main + 1);
    let layers = project.tracks[..drawn]
        .iter()
        .rev()
        .filter(|track| track.kind == TrackKind::Video && !track.hidden)
        .filter_map(|track| active_clip(track, time))
        .filter_map(|clip| layer(project, clip, time, half_frame, canvas))
        .collect();
    RenderPlan {
        frame,
        time,
        canvas,
        render_size,
        background: background(&project.background),
        layers,
    }
}

/// The preview's render size (§8): `canvas` fitted into `bounds` (the preview rectangle times the
/// zoom, in pixels) but never larger than `canvas`, divided by `divisor` (1 full, 2 half,
/// 4 quarter quality). Keeps the canvas aspect, rounds each side down to even, at least 2x2.
/// Integer arithmetic only, so the same inputs always give the same size.
pub fn render_size(canvas: (u32, u32), bounds: (u32, u32), divisor: u32) -> (u32, u32) {
    let (cw, ch) = (u64::from(canvas.0.max(1)), u64::from(canvas.1.max(1)));
    let (bw, bh) = (u64::from(bounds.0), u64::from(bounds.1));
    // Scale factor num/den: the tighter axis, capped at 1.
    let (num, den) = if bw * ch <= bh * cw {
        (bw, cw)
    } else {
        (bh, ch)
    };
    let (num, den) = if num >= den { (1, 1) } else { (num, den) };
    let den = den * u64::from(divisor.max(1));
    let side = |v: u64| {
        u32::try_from(v * num / den / 2 * 2)
            .unwrap_or(u32::MAX - 1)
            .max(2)
    };
    (side(cw), side(ch))
}

/// The clip of `track` active at `t`. Clips are sorted by start and never overlap.
fn active_clip(track: &Track, t: Time) -> Option<&Clip> {
    let started = track.clips.partition_point(|c| c.start <= t);
    track.clips[..started].last().filter(|c| t < c.end())
}

fn layer(
    project: &Project,
    clip: &Clip,
    t: Time,
    half_frame: Time,
    canvas: (u32, u32),
) -> Option<LayerPlan> {
    let (content, source_size) = match &clip.source {
        ClipSource::Color { color } => (LayerContent::Color(*color), None),
        // TODO(0510): text layers.
        ClipSource::Text { .. } => return None,
        // TODO(0370 step 3): `frame_sample_time` for every video layer and a still pace. Until
        // then a freeze frame is a video layer held at its source time.
        ClipSource::Freeze { media, source_time } => match project.media_asset(*media) {
            None => (LayerContent::Placeholder(PlaceholderKind::Offline), None),
            Some(asset) => {
                let info = asset.info.video.as_ref()?;
                let content = LayerContent::Video {
                    media: *media,
                    sample_time: *source_time,
                    convert: ConvertParams::for_video(info),
                };
                (content, Some(info.display_size()))
            }
        },
        ClipSource::Media { media, .. } => match project.media_asset(*media) {
            None => (LayerContent::Placeholder(PlaceholderKind::Offline), None),
            Some(asset) => {
                let info = asset.info.video.as_ref()?;
                let content = match asset.kind {
                    MediaKind::Video => LayerContent::Video {
                        media: *media,
                        sample_time: clip.source_time(t)? + half_frame,
                        convert: ConvertParams::for_video(info),
                    },
                    MediaKind::Image => LayerContent::Image {
                        media: *media,
                        convert: ConvertParams::for_image(info),
                    },
                    MediaKind::Audio => return None,
                };
                (content, Some(info.display_size()))
            }
        },
    };
    let visual = clip.visual.as_ref();
    let fit = visual.map_or(FitMode::Fit, |v| v.fit);
    // Colour cards and placeholders have no shape of their own: they cover the canvas.
    let transform = fit_rect(source_size.unwrap_or(canvas), canvas, fit);
    Some(LayerPlan {
        clip: clip.id,
        content,
        transform,
        crop: CropInsets::default(),
        opacity: visual.map_or(1.0, |v| v.opacity.at(t - clip.start)),
    })
}

/// The centred rectangle a `source`-sized picture occupies on the canvas under `fit`.
fn fit_rect(source: (u32, u32), canvas: (u32, u32), fit: FitMode) -> Affine2 {
    let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
    let (sw, sh) = (f64::from(source.0), f64::from(source.1));
    let (w, h) = if fit == FitMode::Stretch || sw == 0.0 || sh == 0.0 {
        (cw, ch)
    } else {
        let (sx, sy) = (cw / sw, ch / sh);
        let s = if fit == FitMode::Fit {
            sx.min(sy)
        } else {
            sx.max(sy)
        };
        (sw * s, sh * s)
    };
    Affine2::rect(
        ((cw - w) / 2.0) as f32,
        ((ch - h) / 2.0) as f32,
        w as f32,
        h as f32,
    )
}

fn background(background: &CanvasBackground) -> BackgroundPlan {
    match background {
        CanvasBackground::Color { color } => BackgroundPlan::Color(*color),
        // TODO(0350): blurred main-track fill and image backgrounds.
        CanvasBackground::Blur { .. } | CanvasBackground::Image { .. } => {
            BackgroundPlan::Color(Rgba8::BLACK)
        }
    }
}

#[cfg(test)]
mod tests {
    use app_core::media::{MediaAsset, Rotation, samples};
    use app_core::props::VisualProps;
    use app_core::{FrameRate, LinkId, ProjectId, Rational, TrackId};
    use uuid::Uuid;

    use super::*;

    const SECOND: i64 = app_core::FLICKS_PER_SECOND;
    const FULL_HD: (u32, u32) = (1920, 1080);

    fn ms(milliseconds: i64) -> Time {
        Time(milliseconds * SECOND / 1000)
    }

    fn track(id: u128, kind: TrackKind, clips: Vec<Clip>) -> Track {
        Track {
            id: TrackId(Uuid::from_u128(id)),
            clips,
            ..Track::new(kind, format!("Track {id}"))
        }
    }

    fn color_clip(id: u128, start: Time, duration: Time, color: Rgba8) -> Clip {
        Clip {
            id: ClipId(Uuid::from_u128(id)),
            ..Clip::color(format!("Color {id}"), start, duration, color)
        }
    }

    fn media_clip(id: u128, start: Time, duration: Time, media: MediaId, source_in: Time) -> Clip {
        Clip {
            source: ClipSource::media_at(media, source_in),
            ..color_clip(id, start, duration, Rgba8::BLACK)
        }
    }

    fn with_id(mut asset: MediaAsset, id: u128) -> MediaAsset {
        asset.id = MediaId(Uuid::from_u128(id));
        asset
    }

    /// A project with `overlays` above the main track and `below` under it, at `rate`.
    fn project(overlays: Vec<Track>, main: Track, below: Vec<Track>) -> Project {
        let mut project = Project::new("Plan");
        project.id = ProjectId(Uuid::from_u128(1));
        project.main_track = main.id;
        project.tracks = overlays;
        project.tracks.push(main);
        project.tracks.extend(below);
        project
    }

    fn clip_ids(plan: &RenderPlan) -> Vec<u128> {
        plan.layers.iter().map(|l| l.clip.0.as_u128()).collect()
    }

    fn long(seconds: i64) -> Time {
        Time::from_seconds(seconds)
    }

    #[test]
    fn layers_are_main_then_overlays_bottom_to_top() {
        let all = |id| color_clip(id, Time::ZERO, long(10), Rgba8::BLACK);
        let project = project(
            vec![
                track(10, TrackKind::Video, vec![all(100)]),
                track(
                    11,
                    TrackKind::Text,
                    vec![Clip::text("Title", Time::ZERO, long(10), "Hi")],
                ),
                track(12, TrackKind::Video, vec![all(102)]),
            ],
            track(13, TrackKind::Video, vec![all(103)]),
            vec![track(14, TrackKind::Audio, vec![all(104)])],
        );
        let plan = build_plan(&project, long(1), FULL_HD);
        assert_eq!(clip_ids(&plan), [103, 102, 100]);
    }

    #[test]
    fn hidden_tracks_are_skipped() {
        let all = |id| color_clip(id, Time::ZERO, long(10), Rgba8::BLACK);
        let mut hidden_overlay = track(10, TrackKind::Video, vec![all(100)]);
        hidden_overlay.hidden = true;
        let mut project = project(
            vec![hidden_overlay, track(11, TrackKind::Video, vec![all(101)])],
            track(12, TrackKind::Video, vec![all(102)]),
            Vec::new(),
        );
        assert_eq!(
            clip_ids(&build_plan(&project, long(1), FULL_HD)),
            [102, 101]
        );
        project.tracks[2].hidden = true;
        assert_eq!(clip_ids(&build_plan(&project, long(1), FULL_HD)), [101]);
    }

    #[test]
    fn a_clip_is_active_from_its_start_until_before_its_end() {
        let project = project(
            Vec::new(),
            track(
                10,
                TrackKind::Video,
                vec![
                    color_clip(100, Time::ZERO, long(2), Rgba8::BLACK),
                    color_clip(101, long(2), long(2), Rgba8::BLACK),
                ],
            ),
            Vec::new(),
        );
        let at = |t| clip_ids(&build_plan(&project, t, FULL_HD));
        let frame = FrameRate::FPS_30.frame_duration();
        assert_eq!(at(Time::ZERO), [100]);
        assert_eq!(at(long(2) - frame), [100]);
        assert_eq!(at(long(2)), [101], "at a cut point: the next clip");
        assert_eq!(at(long(4) - Time(1)), [101]);
        assert_eq!(at(long(4)), Vec::<u128>::new(), "not at its end");
    }

    #[test]
    fn time_is_quantised_down_to_the_frame_grid() {
        let mut project = project(
            Vec::new(),
            track(10, TrackKind::Video, Vec::new()),
            Vec::new(),
        );
        project.settings.frame_rate = FrameRate::NTSC_30;
        let frame = FrameRate::NTSC_30.frame_duration();
        let plan = |t| {
            let p = build_plan(&project, t, FULL_HD);
            (p.frame, p.time)
        };
        assert_eq!(plan(Time::ZERO), (0, Time::ZERO));
        assert_eq!(plan(frame - Time(1)), (0, Time::ZERO));
        assert_eq!(plan(frame), (1, frame));
        assert_eq!(
            plan(Time(frame.flicks() * 1000 + 7)),
            (1000, Time(frame.flicks() * 1000))
        );
        assert_eq!(plan(Time(-1)), (-1, -frame));

        // A clip that starts inside a frame is not active in that frame.
        project.tracks[0].clips = vec![color_clip(100, Time(10), long(1), Rgba8::BLACK)];
        assert!(build_plan(&project, Time(20), FULL_HD).layers.is_empty());
    }

    /// The source frame each project frame of the first `seconds` shows, from the plan's
    /// `sample_time` and a source with a constant frame period `source_frame`.
    fn shown_source_frames(project_rate: FrameRate, source_frame: Time, seconds: i64) -> Vec<i64> {
        let video = with_id(samples::video(1), 50);
        let media = video.id;
        let mut project = project(
            Vec::new(),
            track(
                10,
                TrackKind::Video,
                vec![media_clip(
                    100,
                    Time::ZERO,
                    long(seconds),
                    media,
                    Time::ZERO,
                )],
            ),
            Vec::new(),
        );
        project.media = vec![video];
        project.settings.frame_rate = project_rate;
        let frames = Time::from_seconds(seconds).to_frames(project_rate);
        (0..frames)
            .map(|n| {
                let plan = build_plan(&project, Time::from_frames(n, project_rate), FULL_HD);
                match plan.layers[0].content {
                    // Sample and hold: the last source frame with pts <= sample_time.
                    LayerContent::Video { sample_time, .. } => {
                        sample_time.flicks().div_euclid(source_frame.flicks())
                    }
                    ref other => panic!("not a video layer: {other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn centre_sampling_shows_every_ntsc_frame_once_in_a_30_fps_project() {
        let shown = shown_source_frames(FrameRate::FPS_30, FrameRate::NTSC_30.frame_duration(), 10);
        assert_eq!(shown.len(), 300);
        assert!(
            shown.iter().enumerate().all(|(n, &k)| k == n as i64),
            "source frames 0, 1, 2, ... without repeats: {shown:?}"
        );
    }

    #[test]
    fn centre_sampling_holds_24_fps_frames_in_a_3_2_pattern_at_60_fps() {
        let shown = shown_source_frames(FrameRate::FPS_60, FrameRate::FPS_24.frame_duration(), 10);
        let mut runs: Vec<(i64, usize)> = Vec::new();
        for k in shown {
            match runs.last_mut() {
                Some((last, count)) if *last == k => *count += 1,
                _ => runs.push((k, 1)),
            }
        }
        assert_eq!(runs.len(), 240, "every source frame of 10 s is shown");
        assert!(runs.iter().enumerate().all(|(i, &(k, _))| k == i as i64));
        let counts: Vec<usize> = runs.iter().map(|&(_, c)| c).collect();
        assert!(
            counts.chunks(2).all(|pair| pair == [2, 3]),
            "2:3 pulldown: {counts:?}"
        );
    }

    #[test]
    fn sample_time_follows_source_in_and_the_clip_start() {
        let video = with_id(samples::video(1), 50);
        let mut project = project(
            Vec::new(),
            track(
                10,
                TrackKind::Video,
                vec![media_clip(100, long(1), long(2), video.id, long(3))],
            ),
            Vec::new(),
        );
        project.media = vec![video];
        let frame = FrameRate::FPS_30.frame_duration();
        let plan = build_plan(&project, long(2) + Time(5), FULL_HD);
        let LayerContent::Video { sample_time, .. } = plan.layers[0].content else {
            panic!("not a video layer");
        };
        assert_eq!(sample_time, long(4) + Time(frame.flicks() / 2));
    }

    #[test]
    fn missing_media_is_an_offline_placeholder_over_the_canvas() {
        let missing = MediaId(Uuid::from_u128(99));
        let audio = with_id(samples::audio(2), 51);
        let mut project = project(
            vec![track(
                10,
                TrackKind::Video,
                vec![media_clip(100, Time::ZERO, long(2), audio.id, Time::ZERO)],
            )],
            track(
                11,
                TrackKind::Video,
                vec![media_clip(101, Time::ZERO, long(2), missing, Time::ZERO)],
            ),
            Vec::new(),
        );
        project.media = vec![audio];
        let plan = build_plan(&project, Time::ZERO, FULL_HD);
        assert_eq!(clip_ids(&plan), [101], "audio-only media draws nothing");
        assert_eq!(
            plan.layers[0].content,
            LayerContent::Placeholder(PlaceholderKind::Offline)
        );
        assert_eq!(
            plan.layers[0].transform,
            Affine2::rect(0.0, 0.0, 1920.0, 1080.0)
        );
    }

    #[test]
    fn fit_modes_use_the_display_size() {
        let mut portrait = with_id(samples::video(1), 50);
        let stream = portrait.info.video.as_mut().unwrap();
        // 1920x1080 coded with a quarter turn: upright 1080x1920.
        stream.rotation = Rotation::Cw90;
        let mut anamorphic = with_id(samples::video(2), 51);
        let stream = anamorphic.info.video.as_mut().unwrap();
        // 1440x1080 with 4:3 pixels: 1920x1080 on screen.
        stream.width = 1440;
        stream.sample_aspect = Rational::new(4, 3).unwrap();
        let rect_for = |asset: &MediaAsset, fit: FitMode| {
            let mut clip = media_clip(100, Time::ZERO, long(1), asset.id, Time::ZERO);
            clip.visual = Some(VisualProps {
                fit,
                ..VisualProps::default()
            });
            let mut project = project(
                Vec::new(),
                track(10, TrackKind::Video, vec![clip]),
                Vec::new(),
            );
            project.media = vec![asset.clone()];
            build_plan(&project, Time::ZERO, FULL_HD).layers[0].transform
        };
        assert_eq!(
            rect_for(&portrait, FitMode::Fit),
            Affine2::rect(656.25, 0.0, 607.5, 1080.0)
        );
        let fill_height = 1920.0_f64 * 1920.0 / 1080.0;
        assert_eq!(
            rect_for(&portrait, FitMode::Fill),
            Affine2::rect(
                0.0,
                ((1080.0 - fill_height) / 2.0) as f32,
                1920.0,
                fill_height as f32
            )
        );
        assert_eq!(
            rect_for(&portrait, FitMode::Stretch),
            Affine2::rect(0.0, 0.0, 1920.0, 1080.0)
        );
        assert_eq!(
            rect_for(&anamorphic, FitMode::Fit),
            Affine2::rect(0.0, 0.0, 1920.0, 1080.0)
        );
        let rect = Affine2::rect(656.25, 0.0, 607.5, 1080.0);
        assert_eq!(rect.apply((0.0, 0.0)), (656.25, 0.0));
        assert_eq!(rect.apply((1.0, 1.0)), (1263.75, 1080.0));
    }

    #[test]
    fn opacity_is_read_and_crop_is_zero() {
        let mut clip = color_clip(100, Time::ZERO, long(1), Rgba8::rgb(1, 2, 3));
        let visual = clip.visual.as_mut().unwrap();
        visual.opacity.value = 0.25;
        visual.crop.value.left = 0.5;
        let project = project(
            Vec::new(),
            track(10, TrackKind::Video, vec![clip]),
            Vec::new(),
        );
        let layer = &build_plan(&project, Time::ZERO, FULL_HD).layers[0];
        assert_eq!(layer.opacity, 0.25);
        assert_eq!(layer.crop, CropInsets::default(), "crop is task 0350");
        assert_eq!(layer.content, LayerContent::Color(Rgba8::rgb(1, 2, 3)));
    }

    #[test]
    fn blur_and_image_backgrounds_fall_back_to_black() {
        let mut project = project(
            Vec::new(),
            track(10, TrackKind::Video, Vec::new()),
            Vec::new(),
        );
        let white = Rgba8::rgb(255, 255, 255);
        project.background = CanvasBackground::Color { color: white };
        assert_eq!(
            build_plan(&project, Time::ZERO, FULL_HD).background,
            BackgroundPlan::Color(white)
        );
        for background in [
            CanvasBackground::Blur { amount: 0.5 },
            CanvasBackground::Image {
                media: MediaId(Uuid::from_u128(5)),
            },
        ] {
            project.background = background;
            assert_eq!(
                build_plan(&project, Time::ZERO, FULL_HD).background,
                BackgroundPlan::Color(Rgba8::BLACK)
            );
        }
    }

    #[test]
    fn render_size_fits_caps_divides_and_rounds_to_even() {
        // Full, half, quarter of a canvas that fits.
        assert_eq!(render_size(FULL_HD, (2560, 1440), 1), (1920, 1080));
        assert_eq!(render_size(FULL_HD, (2560, 1440), 2), (960, 540));
        assert_eq!(render_size(FULL_HD, (2560, 1440), 4), (480, 270));
        // A 4K canvas in a 1280x720 preview composites at 1280x720 at most.
        assert_eq!(render_size((3840, 2160), (1280, 720), 1), (1280, 720));
        assert_eq!(render_size((3840, 2160), (1280, 720), 4), (320, 180));
        // Zoom 200 % of fit: the bounds double, still capped at the canvas.
        assert_eq!(render_size(FULL_HD, (1280 * 2, 720 * 2), 1), (1920, 1080));
        // Odd window sizes, portrait canvas, letterboxing on either axis.
        assert_eq!(render_size(FULL_HD, (1279, 719), 1), (1278, 718));
        assert_eq!(render_size((1080, 1920), (1001, 563), 1), (316, 562));
        assert_eq!(render_size((1080, 1920), (1001, 563), 2), (158, 280));
        assert_eq!(render_size(FULL_HD, (1001, 2000), 1), (1000, 562));
        // Odd canvas, degenerate bounds.
        assert_eq!(render_size((1921, 1081), (4000, 4000), 1), (1920, 1080));
        assert_eq!(render_size(FULL_HD, (0, 0), 1), (2, 2));
        assert_eq!(render_size(FULL_HD, (3, 3), 4), (2, 2));
    }

    /// The canonical project of the snapshot tests: a text track, a hidden overlay, two overlays
    /// (a portrait video with fill, a half-transparent image), the main track (a video clip then a
    /// colour clip) and an audio track. Fixed ids keep the snapshots stable.
    fn canonical() -> Project {
        let mut main_video = with_id(samples::video(1), 0x50);
        main_video.info.video.as_mut().unwrap().pixel_format = "yuv420p10le".into();
        let mut portrait = with_id(samples::video(2), 0x51);
        portrait.info.video.as_mut().unwrap().rotation = Rotation::Cw90;
        let image = with_id(samples::image(3), 0x52);
        let song = with_id(samples::audio(4), 0x53);

        let mut text = Clip::text("Title", Time::ZERO, long(6), "Hello");
        text.id = ClipId(Uuid::from_u128(0x100));
        let mut portrait_clip = media_clip(0x102, ms(1000), long(3), portrait.id, ms(500));
        portrait_clip.visual.as_mut().unwrap().fit = FitMode::Fill;
        let mut image_clip = media_clip(0x103, Time::ZERO, long(5), image.id, Time::ZERO);
        image_clip.visual.as_mut().unwrap().opacity.value = 0.5;
        let mut song_clip = media_clip(0x106, Time::ZERO, long(6), song.id, Time::ZERO);
        song_clip.visual = None;
        let link = LinkId(Uuid::from_u128(0x200));
        let mut main_clip = media_clip(0x104, Time::ZERO, long(4), main_video.id, ms(1250));
        main_clip.link = Some(link);

        let mut hidden = track(
            0x11,
            TrackKind::Video,
            vec![color_clip(
                0x101,
                Time::ZERO,
                long(6),
                Rgba8::rgb(255, 0, 0),
            )],
        );
        hidden.hidden = true;
        let mut project = project(
            vec![
                track(0x10, TrackKind::Text, vec![text]),
                hidden,
                track(0x12, TrackKind::Video, vec![portrait_clip]),
                track(0x13, TrackKind::Video, vec![image_clip]),
            ],
            track(
                0x14,
                TrackKind::Video,
                vec![
                    main_clip,
                    color_clip(0x105, long(4), long(2), Rgba8::rgb(0x20, 0x40, 0x60)),
                ],
            ),
            vec![track(0x15, TrackKind::Audio, vec![song_clip])],
        );
        project.media = vec![main_video, portrait, image, song];
        project.background = CanvasBackground::Color {
            color: Rgba8::rgb(0x10, 0x10, 0x10),
        };
        project
    }

    #[test]
    fn canonical_project_snapshots() {
        let project = canonical();
        let times = [
            ("t0_start", Time::ZERO),
            ("t1_all_overlays_mid_frame", ms(1500) + Time(1234)),
            ("t2_cut_to_colour", long(4)),
            ("t3_colour_only_image_ended", ms(5200)),
            ("t4_after_the_end", long(6)),
        ];
        for (name, t) in times {
            insta::assert_debug_snapshot!(name, build_plan(&project, t, (960, 540)));
        }
    }

    /// Budget §13: `build_plan` for 1,000 clips on 20 tracks in at most 0.2 ms. Timing is only
    /// meaningful in a release build: `cargo test -p engine --release --lib render::plan`.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing needs a release build")]
    fn build_plan_meets_the_budget_for_1000_clips_on_20_tracks() {
        let media = with_id(samples::video(1), 0x50);
        let clip_length = long(2);
        let tracks: Vec<Track> = (0..20u128)
            .map(|t| {
                let clips = (0..50u128)
                    .map(|c| {
                        let start = Time(clip_length.flicks() * c as i64);
                        media_clip(t * 1000 + c, start, clip_length, media.id, Time::ZERO)
                    })
                    .collect();
                track(t, TrackKind::Video, clips)
            })
            .collect();
        let main = tracks[19].clone();
        let mut project = project(tracks[..19].to_vec(), main, Vec::new());
        project.media = vec![media];
        let runs = 2_000;
        let start = std::time::Instant::now();
        let mut layers = 0;
        for i in 0..runs {
            let t = Time(i64::from(i) * 50_000_000);
            layers += std::hint::black_box(build_plan(&project, t, FULL_HD))
                .layers
                .len();
        }
        let per_plan = start.elapsed() / runs;
        println!("build_plan, 1,000 clips on 20 tracks: {per_plan:?} per plan ({layers} layers)");
        assert!(
            per_plan < std::time::Duration::from_micros(200),
            "{per_plan:?} per plan"
        );
    }
}
