//! Opens the committed test clip through the linked FFmpeg (the pinned LGPL DLLs on Windows).

use std::path::Path;

use app_core::Time;
use engine::media::{StreamKind, probe};

const CLIP: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/testsrc-64x64-1s.mp4"
);

#[test]
fn probes_the_test_clip() {
    let info = probe(Path::new(CLIP)).expect("probe the fixture");

    assert!(info.format.contains("mp4"), "{}", info.format);
    assert_eq!(info.duration, Some(Time::from_seconds(1)));
    assert_eq!(info.streams.len(), 2);

    let video = &info.streams[0];
    assert_eq!(video.codec, "mpeg4");
    assert_eq!(
        video.kind,
        StreamKind::Video {
            width: 64,
            height: 64
        }
    );
    assert_eq!(video.duration, Some(Time::from_seconds(1)));

    let audio = &info.streams[1];
    assert_eq!(audio.codec, "aac");
    assert_eq!(
        audio.kind,
        StreamKind::Audio {
            sample_rate: 48_000,
            channels: 1
        }
    );
    assert_eq!(audio.duration, Some(Time::from_seconds(1)));
}

#[test]
fn missing_file_is_an_error_not_a_panic() {
    let err = probe(
        Path::new(CLIP)
            .with_file_name("does-not-exist.mp4")
            .as_path(),
    )
    .unwrap_err();
    assert!(
        matches!(err, engine::media::ProbeError::Open { .. }),
        "{err}"
    );
}
