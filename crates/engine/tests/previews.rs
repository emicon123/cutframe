//! Poster, filmstrip and waveform generators against the committed fixtures
//! (`tests/fixtures/media/README.md`), design `docs/design/media-import.md` §5, §10.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use app_core::MediaAsset;
use app_core::media::{FilmstripLayout, MediaKind, MediaPath, Rotation};
use engine::media::picture::RgbImage;
use engine::media::previews::{
    asset_waveform, filmstrip_tile, filmstrip_tile_image, poster, poster_image,
};
use engine::media::still::decode_tile_grid;
use engine::media::waveform::bucket_count;
use engine::media::{MediaErrorKind, fingerprint, probe};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media")
        .join(name)
}

fn asset_at(path: &Path) -> MediaAsset {
    let probed = probe(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    MediaAsset {
        id: app_core::MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(path).expect("fingerprint"),
        info: probed.info,
    }
}

fn load(name: &str) -> (PathBuf, MediaAsset) {
    let path = fixture(name);
    let asset = asset_at(&path);
    (path, asset)
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn poster_of(name: &str) -> RgbImage {
    let (path, asset) = load(name);
    poster_image(&path, &asset, &no_cancel()).unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn size(image: &RgbImage) -> (u32, u32) {
    (image.width, image.height)
}

/// Mean BT.709 luma (0..=255) and mean saturation (max - min channel, 0..=255).
fn luma_and_saturation(image: &RgbImage) -> (f64, f64) {
    let (mut luma, mut saturation) = (0.0, 0.0);
    for &px in image.data.as_chunks::<3>().0 {
        let [r, g, b] = px.map(f64::from);
        luma += 0.2126 * r + 0.7152 * g + 0.0722 * b;
        saturation += r.max(g).max(b) - r.min(g).min(b);
    }
    let n = (image.data.len() / 3) as f64;
    (luma / n, saturation / n)
}

fn variance(image: &RgbImage) -> f64 {
    let values: Vec<f64> = image.data.iter().map(|&v| f64::from(v)).collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

#[test]
fn posters_have_a_long_side_of_320_and_the_display_shape() {
    for (name, expected) in [
        ("h264-aac.mp4", (320, 320)),
        ("mpeg2-anamorphic.mpg", (320, 180)),
        ("hevc-portrait-cw90-vfr.mov", (320, 320)),
        ("exif-orientation-6.jpg", (240, 320)),
        ("image.png", (320, 240)),
        ("image.webp", (320, 240)),
        ("image.bmp", (320, 240)),
        ("image.tiff", (320, 240)),
        ("animated.gif", (320, 320)),
        ("prores.mov", (320, 320)),
        ("vp9-opus-no-duration.webm", (320, 320)),
        ("av1.mkv", (320, 320)),
        ("ąęłó/zażółć.mp4", (320, 320)),
    ] {
        let image = poster_of(name);
        assert_eq!(size(&image), expected, "{name}");
        assert!(variance(&image) > 10.0, "{name}: the poster is not blank");
    }
}

#[test]
fn rotated_sources_are_turned_upright() {
    for (name, rotation) in [
        ("hevc-portrait-cw90-vfr.mov", Rotation::Cw90),
        ("exif-orientation-6.jpg", Rotation::Cw90),
    ] {
        let (path, asset) = load(name);
        let video = asset.info.video.as_ref().unwrap();
        assert_eq!(video.rotation, rotation, "{name}");
        let upright = poster_image(&path, &asset, &no_cancel()).unwrap();

        // The same frame as stored: the poster is exactly that picture turned clockwise.
        let mut stored = asset.clone();
        stored.info.video.as_mut().unwrap().rotation = Rotation::None;
        let stored = poster_image(&path, &stored, &no_cancel()).unwrap();
        assert_eq!(size(&stored), (upright.height, upright.width), "{name}");
        assert_eq!(stored.rotated(rotation), upright, "{name}");
    }
}

#[test]
fn heic_grids_are_stitched_whole_and_upright() {
    let (path, asset) = load("image-grid.heic");
    assert_eq!(asset.kind, MediaKind::Image);
    let grid = decode_tile_grid(&path, &no_cancel())
        .unwrap()
        .expect("a tile grid");
    // Two 64x64 tiles side by side, as stored.
    assert_eq!(size(&grid), (128, 64));
    let left = grid.cropped(0, 0, 64, 64);
    let right = grid.cropped(64, 0, 64, 64);
    assert!(
        variance(&left) > 10.0 && variance(&right) > 10.0,
        "both tiles decoded"
    );

    // `irot` 90° counter-clockwise: the upright poster is portrait.
    let poster = poster_image(&path, &asset, &no_cancel()).unwrap();
    assert_eq!(size(&poster), (160, 320));
    let expected = grid.resized(320, 160).unwrap().rotated(Rotation::Cw270);
    assert_eq!(poster, expected);

    // Not a grid: nothing to stitch.
    assert_eq!(
        decode_tile_grid(&fixture("image.png"), &no_cancel()).unwrap(),
        None
    );
}

#[test]
fn posters_are_deterministic_jpegs_that_decode_to_their_size() {
    let (path, asset) = load("exif-orientation-6.jpg");
    let jpeg = poster(&path, &asset, &no_cancel()).unwrap();
    assert_eq!(poster(&path, &asset, &no_cancel()).unwrap(), jpeg);
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);

    let dir = std::env::temp_dir().join(format!("cutframe-poster-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("poster.jpg");
    std::fs::write(&file, &jpeg).unwrap();
    let decoded = asset_at(&file);
    assert_eq!(decoded.kind, MediaKind::Image);
    let video = decoded.info.video.unwrap();
    assert_eq!(
        (video.width, video.height),
        (240, 320),
        "upright, no EXIF left to apply"
    );
    assert_eq!(video.rotation, Rotation::None);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn hdr_posters_are_tone_mapped() {
    for name in ["hevc-hlg-dv84.mov", "av1-10bit-pq.mp4"] {
        let (path, asset) = load(name);
        assert!(asset.info.video.as_ref().unwrap().hdr.is_some(), "{name}");
        let image = poster_image(&path, &asset, &no_cancel()).unwrap();
        let (luma, saturation) = luma_and_saturation(&image);
        // Neither crushed nor clipped, and not washed out to grey.
        assert!((40.0..=200.0).contains(&luma), "{name}: mean luma {luma}");
        assert!(saturation > 30.0, "{name}: mean saturation {saturation}");
    }
}

#[test]
fn filmstrip_tiles_hold_their_frames_side_by_side() {
    // 1 s square video: one 96x96 frame.
    let (path, asset) = load("h264-aac.mp4");
    let layout = FilmstripLayout::of(&asset).unwrap();
    assert_eq!((layout.frames, layout.frame_width), (1, 96));
    let tile = filmstrip_tile_image(&path, &asset, 0, &no_cancel()).unwrap();
    assert_eq!(size(&tile), (96, 96));

    // 2 s rotated VFR video: frames at 0 s and 1 s, upright.
    let (path, asset) = load("hevc-portrait-cw90-vfr.mov");
    let layout = FilmstripLayout::of(&asset).unwrap();
    assert_eq!((layout.frames, layout.frame_width), (2, 96));
    let tile = filmstrip_tile_image(&path, &asset, 0, &no_cancel()).unwrap();
    assert_eq!(size(&tile), (192, 96));
    assert_ne!(
        tile.cropped(0, 0, 96, 96),
        tile.cropped(96, 0, 96, 96),
        "two different frames"
    );
    let mut stored = asset.clone();
    stored.info.video.as_mut().unwrap().rotation = Rotation::None;
    let stored = filmstrip_tile_image(&path, &stored, 0, &no_cancel()).unwrap();
    assert_eq!(
        tile.cropped(0, 0, 96, 96),
        stored.cropped(0, 0, 96, 96).rotated(Rotation::Cw90),
        "filmstrip frames are turned upright too"
    );

    // Anamorphic 4:3 pixels: 192x108 on screen gives 171 px wide frames.
    let (path, asset) = load("mpeg2-anamorphic.mpg");
    let layout = FilmstripLayout::of(&asset).unwrap();
    let tile = filmstrip_tile_image(&path, &asset, 0, &no_cancel()).unwrap();
    assert_eq!(size(&tile), (171 * layout.frames, 96));
    for _ in 0..5 {
        assert_eq!(
            filmstrip_tile_image(&path, &asset, 0, &no_cancel()).unwrap(),
            tile
        );
    }
    let jpeg = filmstrip_tile(&path, &asset, 0, &no_cancel()).unwrap();
    assert_eq!(
        filmstrip_tile(&path, &asset, 0, &no_cancel()).unwrap(),
        jpeg
    );

    // Past the last tile, and kinds without a filmstrip.
    let err = filmstrip_tile_image(&path, &asset, 1, &no_cancel()).unwrap_err();
    assert_eq!(err.kind, MediaErrorKind::UnsupportedFormat);
    let (path, image) = load("image.png");
    assert!(filmstrip_tile_image(&path, &image, 0, &no_cancel()).is_err());
}

/// A mono 16-bit PCM WAV file of `samples`.
fn write_wav(path: &Path, rate: u32, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn waveforms_follow_the_signal() {
    let dir = std::env::temp_dir().join(format!("cutframe-waveform-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let rate = 48_000;
    let sine: Vec<i16> = (0..rate / 2)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * 1000.0 * f64::from(i) / f64::from(rate);
            (phase.sin() * 32_767.0).round() as i16
        })
        .collect();
    let sine_path = dir.join("sine-1k.wav");
    write_wav(&sine_path, rate, &sine);
    let silence_path = dir.join("silence.wav");
    write_wav(&silence_path, rate, &vec![0; rate as usize / 2]);

    let full = asset_at(&sine_path);
    let waveform = asset_waveform(&sine_path, &full, &no_cancel()).unwrap();
    assert_eq!(
        waveform.buckets.len(),
        50,
        "0.5 s at 100 buckets per second"
    );
    assert!(
        waveform
            .buckets
            .iter()
            .all(|&(min, max)| min <= -125 && max >= 125),
        "full-scale sine peaks near ±127"
    );

    let silent = asset_at(&silence_path);
    let waveform = asset_waveform(&silence_path, &silent, &no_cancel()).unwrap();
    assert!(waveform.buckets.iter().all(|&b| b == (0, 0)));
    std::fs::remove_dir_all(dir).ok();

    // The fixtures' sine is FFmpeg's default amplitude, 1/8 of full scale: about ±16 in mono.
    for name in [
        "stereo-44k.mp3",
        "mono-48k.wav",
        "stereo-96k.flac",
        "opus.ogg",
        "aac.m4a",
    ] {
        let (path, asset) = load(name);
        let waveform = asset_waveform(&path, &asset, &no_cancel()).unwrap();
        let expected = bucket_count(asset.info.duration.unwrap());
        assert_eq!(waveform.buckets.len(), expected, "{name}");
        // The median bucket: codecs overshoot at the start (AAC priming).
        let mut peaks: Vec<i8> = waveform.buckets.iter().map(|&(_, max)| max).collect();
        peaks.sort_unstable();
        let peak = peaks[peaks.len() / 2];
        // Stereo fixtures were upmixed from mono at -3 dB (FFmpeg's `-ac 2`): about ±11.
        assert!((10..=19).contains(&peak), "{name}: peak {peak}");
    }

    // A video's waveform comes from its default audio stream; images have none.
    let (path, video) = load("h264-aac.mp4");
    let waveform = asset_waveform(&path, &video, &no_cancel()).unwrap();
    assert_eq!(waveform.buckets.len(), 100);
    let (path, image) = load("image.png");
    assert!(asset_waveform(&path, &image, &no_cancel()).is_err());
}

#[test]
fn a_set_cancel_flag_stops_generation() {
    let cancel = AtomicBool::new(true);
    let (path, asset) = load("hevc-portrait-cw90-vfr.mov");
    let err = poster_image(&path, &asset, &cancel).unwrap_err();
    assert_eq!(err.kind, MediaErrorKind::Cancelled);
    let err = asset_waveform(&path, &load("aac.m4a").1, &cancel).unwrap_err();
    assert_eq!(err.kind, MediaErrorKind::Cancelled);
}

#[test]
fn a_missing_file_is_reported_as_not_found() {
    let (_, asset) = load("h264-aac.mp4");
    let err = poster_image(Path::new("/no/such/file.mp4"), &asset, &no_cancel()).unwrap_err();
    assert_eq!(err.kind, MediaErrorKind::FileNotFound);
}
