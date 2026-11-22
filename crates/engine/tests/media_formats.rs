//! Format coverage of the probe, the fingerprint and the VFR timing rule against the committed
//! fixtures (`tests/fixtures/media/README.md`), through the linked FFmpeg (the pinned LGPL DLLs on
//! Windows). Design `docs/design/media-import.md` §10.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use app_core::media::{HdrTransfer, MediaKind, Rotation, validate_asset};
use app_core::{FLICKS_PER_SECOND, Rational, Time};
use engine::media::decode::{frame_index_at, video_frame_times};
use engine::media::{MediaErrorKind, Probed, fingerprint, probe, probe_cancellable};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media")
        .join(name)
}

fn probed(name: &str) -> Probed {
    let path = fixture(name);
    let probed = probe(&path).unwrap_or_else(|err| panic!("{name}: {err}"));
    // Every probe result must be a valid model asset.
    let asset = app_core::MediaAsset {
        id: app_core::MediaId::new(),
        kind: probed.kind,
        path: app_core::media::MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(&path).expect("fingerprint"),
        info: probed.info.clone(),
    };
    assert_eq!(validate_asset(&asset), Ok(()), "{name}: {probed:#?}");
    probed
}

fn rational(num: i64, den: i64) -> Rational {
    Rational::new(num, den).unwrap()
}

fn seconds_approx(time: Option<Time>, expected: f64, name: &str) {
    let got = time.expect("duration").as_seconds_f64();
    assert!(
        (got - expected).abs() < 0.06,
        "{name}: duration {got} s, expected {expected} s"
    );
}

fn error_kind(name: &str) -> MediaErrorKind {
    match probe(&fixture(name)) {
        Ok(probed) => panic!("{name} probed as {probed:#?}"),
        Err(err) => err.kind,
    }
}

#[test]
fn h264_aac_mp4() {
    let p = probed("h264-aac.mp4");
    assert_eq!(p.kind, MediaKind::Video);
    assert_eq!(p.info.container, "mov");
    assert_eq!(p.info.origin, Time::ZERO);
    seconds_approx(p.info.duration, 1.0, "h264-aac.mp4");
    let video = p.info.video.unwrap();
    assert_eq!(video.codec, "h264");
    assert_eq!((video.width, video.height), (64, 64));
    assert_eq!(video.frame_rate, Some(rational(30, 1)));
    assert_eq!(video.rotation, Rotation::None);
    assert_eq!(
        (video.pixel_format.as_str(), video.bit_depth),
        ("yuv420p", 8)
    );
    // SDR: no HDR info.
    assert_eq!(video.hdr, None);
    assert_eq!(p.info.audio.len(), 1);
    let audio = &p.info.audio[0];
    assert_eq!(
        (audio.codec.as_str(), audio.sample_rate, audio.channels),
        ("aac", 48_000, 2)
    );
}

#[test]
fn portrait_vfr_hevc_is_upright_and_timed_by_timestamps() {
    let name = "hevc-portrait-cw90-vfr.mov";
    let p = probed(name);
    assert_eq!(p.kind, MediaKind::Video);
    let video = p.info.video.as_ref().unwrap();
    assert_eq!(video.codec, "hevc");
    assert_eq!(video.rotation, Rotation::Cw90);
    assert_eq!(video.display_size(), (64, 64));
    seconds_approx(p.info.duration, 2.0, name);

    // Frames 10..20 of 60 at 30 fps were dropped: the kept frames sit at n/30 s.
    let times = video_frame_times(&fixture(name), &p.info).unwrap();
    let kept: Vec<i64> = (0..60).filter(|n| !(10..20).contains(n)).collect();
    let expected: Vec<Time> = kept
        .iter()
        .map(|n| Time::from_flicks(n * FLICKS_PER_SECOND / 30))
        .collect();
    assert_eq!(times, expected);
    assert!(times.windows(2).all(|w| w[0] < w[1]), "monotonic");

    // Sample and hold at 20 probe times spread over the clip, including inside the gap.
    for step in 0..20 {
        let t = Time::from_flicks(step * FLICKS_PER_SECOND / 10 + 1);
        let frame_number = t.flicks() * 30 / FLICKS_PER_SECOND; // the CFR frame at t
        let shown = kept.iter().rposition(|&n| n <= frame_number);
        assert_eq!(frame_index_at(&times, t), shown, "t = {t:?}");
    }
}

#[test]
fn anamorphic_mpeg2_is_shown_wide() {
    let p = probed("mpeg2-anamorphic.mpg");
    let video = p.info.video.unwrap();
    assert_eq!(video.codec, "mpeg2video");
    assert_eq!((video.width, video.height), (144, 108));
    assert_eq!(video.sample_aspect, rational(4, 3));
    assert_eq!(video.display_size(), (192, 108));
}

#[test]
fn webm_without_container_duration_is_timed_by_its_last_packet() {
    let name = "vp9-opus-no-duration.webm";
    let p = probed(name);
    assert_eq!(p.kind, MediaKind::Video);
    assert_eq!(p.info.container, "matroska");
    assert_eq!(p.info.video.as_ref().unwrap().codec, "vp9");
    assert_eq!(p.info.audio[0].codec, "opus");
    seconds_approx(p.info.duration, 1.0, name);
}

#[test]
fn other_video_codecs_probe_and_decode() {
    for (name, codec, container) in [
        ("vp8.webm", "vp8", "matroska"),
        ("av1.mkv", "av1", "matroska"),
        ("prores.mov", "prores", "mov"),
        ("mjpeg.avi", "mjpeg", "avi"),
        ("mpeg4.m4v", "mpeg4", "mov"),
        ("ąęłó/zażółć.mp4", "mpeg4", "mov"),
    ] {
        let p = probed(name);
        assert_eq!(p.kind, MediaKind::Video, "{name}");
        assert_eq!(p.info.container, container, "{name}");
        assert_eq!(p.info.video.unwrap().codec, codec, "{name}");
        assert!(p.info.duration.unwrap() > Time::ZERO, "{name}");
    }
}

#[test]
fn every_audio_track_is_listed_default_first() {
    let p = probed("two-audio-tracks.mkv");
    let audio: Vec<_> = p
        .info
        .audio
        .iter()
        .map(|a| (a.stream_index, a.sample_rate, a.channels))
        .collect();
    assert_eq!(audio, [(2, 44_100, 1), (1, 48_000, 2)]);
}

#[test]
fn hlg_with_dolby_vision_8_4() {
    let p = probed("hevc-hlg-dv84.mov");
    let video = p.info.video.unwrap();
    assert_eq!(video.color.transfer.as_deref(), Some("arib-std-b67"));
    assert_eq!(video.color.primaries.as_deref(), Some("bt2020"));
    assert_eq!(video.color.matrix.as_deref(), Some("bt2020nc"));
    assert_eq!(video.color.range.as_deref(), Some("tv"));
    let hdr = video.hdr.expect("HDR");
    assert_eq!(hdr.transfer, HdrTransfer::Hlg);
    let dv = hdr.dolby_vision.expect("Dolby Vision record");
    assert_eq!((dv.profile, dv.level, dv.compatibility_id), (8, 4, 4));
}

#[test]
fn ten_bit_pq_with_mastering_metadata() {
    let p = probed("av1-10bit-pq.mp4");
    let video = p.info.video.unwrap();
    assert_eq!(
        (video.pixel_format.as_str(), video.bit_depth),
        ("yuv420p10le", 10)
    );
    let hdr = video.hdr.expect("HDR");
    assert_eq!(hdr.transfer, HdrTransfer::Pq);
    assert_eq!(hdr.mastering_max_nits, Some(1000));
    assert_eq!(hdr.max_cll_nits, Some(400));
    assert_eq!(hdr.dolby_vision, None);
}

#[test]
fn audio_formats() {
    for (name, codec, rate, channels, seconds) in [
        ("stereo-44k.mp3", "mp3", 44_100, 2, 0.5),
        ("mono-48k.wav", "pcm_s16le", 48_000, 1, 0.5),
        ("stereo-96k.flac", "flac", 96_000, 2, 0.5),
        ("opus.ogg", "opus", 48_000, 2, 0.5),
        ("aac.m4a", "aac", 44_100, 1, 0.5),
    ] {
        let p = probed(name);
        assert_eq!(p.kind, MediaKind::Audio, "{name}");
        assert!(p.info.video.is_none(), "{name}");
        let audio = &p.info.audio[0];
        assert_eq!(
            (audio.codec.as_str(), audio.sample_rate, audio.channels),
            (codec, rate, channels),
            "{name}"
        );
        seconds_approx(p.info.duration, seconds, name);
    }
}

#[test]
fn still_images() {
    for (name, codec, size) in [
        ("image.png", "png", (64, 48)),
        ("image.webp", "webp", (64, 48)),
        ("image.bmp", "bmp", (64, 48)),
        ("image.tiff", "tiff", (64, 48)),
    ] {
        let p = probed(name);
        assert_eq!(p.kind, MediaKind::Image, "{name}");
        assert_eq!(p.info.duration, None, "{name}");
        let video = p.info.video.unwrap();
        assert_eq!(video.codec, codec, "{name}");
        assert_eq!(video.display_size(), size, "{name}");
        assert_eq!(video.frame_rate, None, "{name}");
    }
}

#[test]
fn jpeg_exif_orientation_is_honoured() {
    let p = probed("exif-orientation-6.jpg");
    assert_eq!(p.kind, MediaKind::Image);
    let video = p.info.video.unwrap();
    assert_eq!(video.codec, "mjpeg");
    assert_eq!(video.rotation, Rotation::Cw90);
    assert_eq!(video.display_size(), (48, 64));
}

#[test]
fn animated_gif_is_video() {
    let p = probed("animated.gif");
    assert_eq!(p.kind, MediaKind::Video);
    assert_eq!(p.info.video.unwrap().codec, "gif");
    assert!(p.info.duration.unwrap() > Time::ZERO);
}

#[test]
fn heic_tile_grid_is_one_rotated_image() {
    let p = probed("image-grid.heic");
    assert_eq!(p.kind, MediaKind::Image);
    let video = p.info.video.unwrap();
    assert_eq!(video.codec, "hevc");
    // A 2x1 grid of 64x64 tiles, turned 90 degrees counter-clockwise by `irot`.
    assert_eq!((video.width, video.height), (128, 64));
    assert_eq!(video.rotation, Rotation::Cw270);
    assert_eq!(video.display_size(), (64, 128));
}

/// An OBS-style MKV whose recording crashed: no stated duration, cut off mid-cluster. The good
/// part imports; its duration is the end of the last good packet. (FFmpeg 7.1's Matroska demuxer
/// stops cleanly at the cut; a tail that does return a read error is handled by `tail_error`,
/// unit-tested in `probe.rs`.)
#[test]
fn crashed_mkv_recording_imports_its_good_part() {
    let p = probed("crashed-recording.mkv");
    assert_eq!(p.kind, MediaKind::Video);
    assert_eq!(p.info.container, "matroska");
    let seconds = p.info.duration.unwrap().as_seconds_f64();
    assert!((0.9..1.5).contains(&seconds), "duration {seconds} s");
}

#[test]
fn broken_files_are_rejected_without_panicking() {
    assert_eq!(error_kind("truncated.mp4"), MediaErrorKind::Damaged);
    assert_eq!(
        error_kind("random-bytes.mp4"),
        MediaErrorKind::UnsupportedFormat
    );
    assert_eq!(error_kind("text.mov"), MediaErrorKind::UnsupportedFormat);
    assert_eq!(error_kind("empty.mp4"), MediaErrorKind::UnsupportedFormat);
    assert_eq!(
        error_kind("does-not-exist.mp4"),
        MediaErrorKind::FileNotFound
    );
}

#[test]
fn a_set_cancel_flag_stops_the_probe() {
    let cancel = AtomicBool::new(true);
    let err = probe_cancellable(&fixture("h264-aac.mp4"), &cancel).unwrap_err();
    assert_eq!(err.kind, MediaErrorKind::Cancelled);
}

#[test]
fn fingerprints_identify_content() {
    let a = fingerprint(&fixture("h264-aac.mp4")).unwrap();
    let b = fingerprint(&fixture("h264-aac.mp4")).unwrap();
    let c = fingerprint(&fixture("av1.mkv")).unwrap();
    assert_eq!(a.hash, b.hash);
    assert_ne!(a.hash, c.hash);
    assert_eq!(
        a.size,
        std::fs::metadata(fixture("h264-aac.mp4")).unwrap().len()
    );
}

/// The fixture from task 0200 (MPEG-4 Part 2 + mono AAC).
#[test]
fn probes_the_test_clip() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/testsrc-64x64-1s.mp4");
    let p = probe(&path).unwrap();
    assert_eq!(p.kind, MediaKind::Video);
    assert_eq!(p.info.duration, Some(Time::from_seconds(1)));
    assert_eq!(p.info.video.unwrap().codec, "mpeg4");
    assert_eq!(p.info.audio[0].channels, 1);
}
