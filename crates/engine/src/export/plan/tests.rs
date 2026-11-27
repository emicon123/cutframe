use app_core::{Clip, Rgba8, Track, TrackKind};

use super::*;

fn rate(num: u32, den: u32) -> FrameRate {
    FrameRate::new(num, den).unwrap()
}

/// A project with one colour clip of `duration` on a `w x h` canvas.
fn project(w: u32, h: u32, frame_rate: FrameRate, sample_rate: u32, duration: Time) -> Project {
    let mut project = Project::new("Export");
    project.settings.width = w;
    project.settings.height = h;
    project.settings.frame_rate = frame_rate;
    project.settings.sample_rate = sample_rate;
    let mut track = Track::new(TrackKind::Video, "Video 1");
    if duration > Time::ZERO {
        track
            .clips
            .push(Clip::color("Card", Time::ZERO, duration, Rgba8::BLACK));
    }
    project.tracks.push(track);
    project
}

fn settings(resolution: ExportResolution, quality: ExportQuality) -> VideoExportSettings {
    VideoExportSettings {
        codec: VideoCodec::H264,
        resolution,
        frame_rate: None,
        quality,
        encoder: None,
    }
}

const ALL_RESOLUTIONS: [ExportResolution; 6] = [
    ExportResolution::Project,
    ExportResolution::P480,
    ExportResolution::P720,
    ExportResolution::P1080,
    ExportResolution::P1440,
    ExportResolution::P2160,
];

#[test]
fn sizes_keep_the_canvas_aspect_for_every_class() {
    use ExportResolution as R;
    let cases = [
        // landscape 16:9
        ((1920, 1080), R::Project, (1920, 1080)),
        ((1920, 1080), R::P480, (854, 480)),
        ((1920, 1080), R::P720, (1280, 720)),
        ((1920, 1080), R::P1080, (1920, 1080)),
        ((1920, 1080), R::P1440, (2560, 1440)),
        ((1920, 1080), R::P2160, (3840, 2160)),
        // portrait 9:16 (Shorts)
        ((1080, 1920), R::P480, (480, 854)),
        ((1080, 1920), R::P720, (720, 1280)),
        ((1080, 1920), R::P1080, (1080, 1920)),
        ((1080, 1920), R::P2160, (2160, 3840)),
        // square
        ((1080, 1080), R::P720, (720, 720)),
        ((1080, 1080), R::P2160, (2160, 2160)),
        // odd ratios round to the nearest even number: 4:3 at 1080 is 1440 x 1080; 21:9-ish
        ((2560, 1080), R::P720, (1706, 720)),
        ((1000, 750), R::P480, (640, 480)),
        ((1366, 768), R::P720, (1280, 720)),
        ((1366, 768), R::P480, (854, 480)),
        // upscaling is allowed
        ((1280, 720), R::P2160, (3840, 2160)),
        // a tiny canvas still gives at least 16 px per side
        ((16, 4096), R::P480, (480, 122880)),
    ];
    for ((w, h), resolution, expected) in cases {
        assert_eq!(
            output_size(w, h, resolution),
            expected,
            "{w} x {h} at {resolution:?}"
        );
    }
}

#[test]
fn sizes_are_always_even_and_at_least_16() {
    for (w, h) in [
        (16, 16),
        (18, 8192),
        (1918, 1078),
        (4096, 2160),
        (1334, 750),
    ] {
        for resolution in ALL_RESOLUTIONS.into_iter().skip(1) {
            let (ow, oh) = output_size(w, h, resolution);
            assert!(ow % 2 == 0 && oh % 2 == 0, "{w}x{h} {resolution:?}");
            assert!(ow >= 16 && oh >= 16, "{w}x{h} {resolution:?}");
            assert_eq!(ow.min(oh), resolution.short_side().unwrap());
        }
    }
}

#[test]
fn frame_rate_rules() {
    let fps_50 = rate(50, 1);
    // default: the project rate, or 60 above 60
    assert_eq!(
        export_frame_rate(FrameRate::NTSC_30, None),
        Ok(FrameRate::NTSC_30)
    );
    assert_eq!(export_frame_rate(rate(120, 1), None), Ok(FrameRate::FPS_60));
    assert_eq!(export_frame_rate(rate(60, 1), None), Ok(FrameRate::FPS_60));
    // the fixed rates, from any project rate
    for fixed in [
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::FPS_30,
        fps_50,
        FrameRate::FPS_60,
    ] {
        assert_eq!(export_frame_rate(rate(120, 1), Some(fixed)), Ok(fixed));
    }
    // 29.97 only as the project rate
    assert_eq!(
        export_frame_rate(FrameRate::NTSC_30, Some(FrameRate::NTSC_30)),
        Ok(FrameRate::NTSC_30)
    );
    assert_eq!(
        export_frame_rate(FrameRate::FPS_30, Some(FrameRate::NTSC_30)),
        Err(PlanError::InvalidFrameRate {
            num: 30000,
            den: 1001
        })
    );
    // the project rate above 60 is not allowed explicitly
    assert_eq!(
        export_frame_rate(rate(120, 1), Some(rate(120, 1))),
        Err(PlanError::InvalidFrameRate { num: 120, den: 1 })
    );
    // an unreduced fraction is the same rate
    assert_eq!(
        export_frame_rate(FrameRate::FPS_24, Some(rate(60, 2))),
        Ok(rate(60, 2))
    );
}

#[test]
fn frames_round_up_to_cover_the_timeline() {
    let f = FrameRate::FPS_30.frame_duration();
    let plan = |d: Time| {
        plan_video_export(
            &project(1920, 1080, FrameRate::FPS_30, 48_000, d),
            &settings(ExportResolution::Project, ExportQuality::Recommended),
        )
        .unwrap()
    };
    assert_eq!(plan(Time::from_seconds(10)).frames, 300);
    assert_eq!(plan(Time::from_flicks(1)).frames, 1);
    assert_eq!(plan(Time::from_flicks(f.flicks() * 300 + 1)).frames, 301);
    assert_eq!(plan(Time::from_flicks(f.flicks() * 300 - 1)).frames, 300);
    assert_eq!(
        plan(Time::from_seconds(10)).duration(),
        Time::from_seconds(10)
    );
}

#[test]
fn samples_end_with_the_last_video_frame() {
    let rates = [
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::NTSC_30,
        FrameRate::FPS_30,
        rate(50, 1),
        FrameRate::FPS_60,
    ];
    for frame_rate in rates {
        for sample_rate in [44_100, 48_000] {
            for duration in [
                Time::from_seconds(1),
                Time::from_flicks(1_234_567_891),
                Time::from_seconds(600),
            ] {
                let plan = plan_video_export(
                    &project(1920, 1080, frame_rate, sample_rate, duration),
                    &settings(ExportResolution::Project, ExportQuality::Recommended),
                )
                .unwrap();
                let video = i128::from(plan.duration().flicks());
                let audio = i128::from(plan.samples) * i128::from(FLICKS_PER_SECOND);
                let sample = i128::from(FLICKS_PER_SECOND);
                let video_x_rate = video * i128::from(sample_rate);
                // never past the video, and less than one sample short of it
                assert!(audio <= video_x_rate, "{frame_rate:?} {sample_rate}");
                assert!(
                    video_x_rate - audio < sample,
                    "{frame_rate:?} {sample_rate}"
                );
            }
        }
    }
    let plan = plan_video_export(
        &project(
            1920,
            1080,
            FrameRate::FPS_30,
            48_000,
            Time::from_seconds(10),
        ),
        &settings(ExportResolution::Project, ExportQuality::Recommended),
    )
    .unwrap();
    assert_eq!(plan.samples, 480_000);
}

#[test]
fn bitrate_table_and_factors() {
    use ExportQuality::{High, Low, Recommended};
    use VideoCodec::{H264, Hevc};
    let fps_30 = FrameRate::FPS_30;
    let fps_60 = FrameRate::FPS_60;
    let cases = [
        ((1920, 1080), fps_30, Recommended, H264, 12_000),
        ((1920, 1080), fps_30, Low, H264, 8_000),
        ((1920, 1080), fps_30, High, H264, 20_000),
        ((1920, 1080), fps_60, Recommended, H264, 18_000),
        ((1920, 1080), FrameRate::NTSC_30, Recommended, H264, 12_000),
        ((3840, 2160), fps_30, Recommended, H264, 67_500),
        ((3840, 2160), fps_60, High, H264, 170_000),
        ((1080, 1920), fps_30, Recommended, H264, 12_000),
        // 8000 x 1.5 x 1080^2 / (1920 x 1080) = 6750, rounded half up
        ((1080, 1080), fps_30, Recommended, H264, 7_000),
        // 8000 x 1.5 x 0.6 = 7200
        ((1920, 1080), fps_30, Recommended, Hevc, 7_000),
        ((2560, 1440), fps_30, Recommended, H264, 24_000),
        ((1280, 720), fps_30, Recommended, H264, 7_500),
        ((854, 480), fps_30, Recommended, H264, 4_000),
        // below the smallest class: the 480 class, scaled down by area, at least 500
        ((640, 360), fps_30, Low, H264, 1_500),
        ((16, 16), fps_30, Low, H264, 500),
        // 8K: clamped to the custom maximum
        ((7680, 4320), fps_60, High, H264, 200_000),
    ];
    for ((w, h), frame_rate, quality, codec, expected) in cases {
        assert_eq!(
            video_kbps(w, h, frame_rate, quality, codec),
            expected,
            "{w}x{h} {frame_rate:?} {quality:?} {codec:?}"
        );
    }
}

#[test]
fn custom_bitrate_range() {
    let p = project(1920, 1080, FrameRate::FPS_30, 48_000, Time::from_seconds(1));
    let custom = |kbps| {
        plan_video_export(
            &p,
            &settings(ExportResolution::P1080, ExportQuality::Custom { kbps }),
        )
    };
    assert_eq!(custom(500).unwrap().video_kbps, 500);
    assert_eq!(custom(200_000).unwrap().video_kbps, 200_000);
    assert_eq!(custom(12_345).unwrap().video_kbps, 12_345);
    assert_eq!(custom(499), Err(PlanError::InvalidBitrate { kbps: 499 }));
    assert_eq!(
        custom(200_001),
        Err(PlanError::InvalidBitrate { kbps: 200_001 })
    );
}

#[test]
fn estimate_gop_and_audio() {
    let plan = plan_video_export(
        &project(
            1920,
            1080,
            FrameRate::FPS_30,
            48_000,
            Time::from_seconds(60),
        ),
        &settings(ExportResolution::P1080, ExportQuality::Recommended),
    )
    .unwrap();
    assert_eq!(plan.audio_kbps, 384);
    assert_eq!(plan.sample_rate, 48_000);
    assert_eq!(plan.gop, 15);
    // (12000 + 384) kbps x 125 x 60 s x 1.05
    assert_eq!(plan.estimated_bytes, 97_524_000);

    assert_eq!(gop(FrameRate::FPS_24), 12);
    assert_eq!(gop(FrameRate::FPS_25), 13);
    assert_eq!(gop(FrameRate::NTSC_30), 15);
    assert_eq!(gop(FrameRate::FPS_60), 30);
    assert_eq!(gop(rate(1, 1)), 1);
}

#[test]
fn empty_timeline_is_an_error() {
    let p = project(1920, 1080, FrameRate::FPS_30, 48_000, Time::ZERO);
    assert_eq!(
        plan_video_export(
            &p,
            &settings(ExportResolution::P1080, ExportQuality::Recommended)
        ),
        Err(PlanError::EmptyTimeline)
    );
}

#[test]
fn a_different_export_rate_uses_its_own_frame_grid() {
    let p = project(1920, 1080, FrameRate::FPS_60, 48_000, Time::from_seconds(2));
    let plan = plan_video_export(
        &p,
        &VideoExportSettings {
            frame_rate: Some(FrameRate::FPS_30),
            ..settings(ExportResolution::P1080, ExportQuality::Recommended)
        },
    )
    .unwrap();
    assert_eq!(plan.frame_rate, FrameRate::FPS_30);
    assert_eq!(plan.frames, 60);
    assert_eq!(plan.video_kbps, 12_000);
}
