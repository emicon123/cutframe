//! The default output device (WASAPI shared mode): plays 2 s of the 1 kHz sine fixture (two
//! clips with a split point) through the feeder and reports the callback load, underruns, the
//! device format and the latency. Needs a sound device, so it is ignored by default:
//!
//! ```text
//! cargo test -p engine --test audio_device -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::media::MediaPath;
use app_core::model::{ClipId, ClipSource};
use app_core::{AudioProps, Clip, MediaAsset, MediaId, Project, Time};
use arc_swap::ArcSwap;
use engine::audio::AudioEngine;
use engine::media::{fingerprint, probe};
use engine::transport::{PlayState, TransportShared};

#[test]
#[ignore = "opens the default audio output device"]
fn plays_two_seconds_on_the_default_device() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media/sine-1k-48k.flac");
    let probed = probe(&path).unwrap();
    let asset = MediaAsset {
        id: MediaId::new(),
        kind: probed.kind,
        path: MediaPath::absolute(path.to_string_lossy()),
        fingerprint: fingerprint(&path).unwrap(),
        info: probed.info,
    };
    let clip = |start: i64| Clip {
        id: ClipId::new(),
        name: "sine".into(),
        start: Time::from_seconds(start),
        duration: Time::from_seconds(1),
        source: ClipSource::media_at(asset.id, Time::ZERO),
        link: None,
        visual: None,
        audio: Some(AudioProps {
            volume: app_core::Animated::constant(0.2),
            ..AudioProps::default()
        }),
    };
    let mut project = Project::new("device");
    project.tracks[1].clips = vec![clip(0), clip(1)];
    project.media.push(asset);
    let transport = Arc::new(TransportShared::new());
    let engine = AudioEngine::new(
        Arc::clone(&transport),
        Arc::new(ArcSwap::from_pointee(project)),
        |event| println!("event: {event:?}"),
    )
    .unwrap();
    let clock = engine.clock();
    // The app starts the engine long before the first Space press: let the feeder warm up.
    std::thread::sleep(Duration::from_millis(500));

    let pressed = Instant::now();
    let seek_gen = transport.seek(Time::ZERO);
    transport.set_state(PlayState::Prerolling);
    engine.play(Time::ZERO, 1, seek_gen);
    while !engine.ring_ready() && pressed.elapsed() < Duration::from_millis(300) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let preroll = pressed.elapsed();
    transport.set_started(true);
    transport.set_state(PlayState::Playing);
    let first = loop {
        if let Some(reading) = clock.now() {
            break reading;
        }
        assert!(
            pressed.elapsed() < Duration::from_secs(2),
            "the clock never started"
        );
        std::thread::sleep(Duration::from_micros(500));
    };
    let to_sound = pressed.elapsed();
    let end = loop {
        let reading = clock.now().expect("a reading while playing");
        if reading.ended {
            break reading;
        }
        assert!(
            pressed.elapsed() < Duration::from_secs(5),
            "never ended: {reading:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let played = pressed.elapsed() - to_sound;
    engine.pause();

    let output = engine.output().expect("an open output");
    let load = engine.callback_load();
    println!("output: {output:?}");
    println!("reported latency: {:?}", engine.latency());
    println!("preroll (ring ready): {preroll:?}; play to first audible sample: {to_sound:?}");
    println!(
        "first reading: {first:?}; end: {end:?}; wall time from first sample to end: {played:?}"
    );
    println!(
        "callback load: average {:.3} %, max {:.3} % of the buffer period over {} calls",
        load.average_percent, load.max_percent, load.calls
    );
    println!("underruns: {}", engine.underruns());
    assert!(output.device, "no device: the null output ran");
    assert_eq!(end.time, Time::from_seconds(2));
    assert_eq!(engine.underruns(), 0);
    let drift = (played.as_secs_f64() - 2.0).abs();
    assert!(drift < 0.05, "2 s of audio took {played:?}");

    // Again with the stream open (paused less than 10 s ago).
    transport.set_started(false);
    let pressed = Instant::now();
    let seek_gen = transport.seek(Time::ZERO);
    engine.play(Time::ZERO, 1, seek_gen);
    while !engine.ring_ready() && pressed.elapsed() < Duration::from_millis(300) {
        std::thread::sleep(Duration::from_micros(200));
    }
    let warm = pressed.elapsed();
    transport.set_started(true);
    while clock.now().is_none() {
        assert!(
            pressed.elapsed() < Duration::from_secs(2),
            "the clock never started"
        );
        std::thread::sleep(Duration::from_micros(200));
    }
    println!(
        "warm: ring ready after {warm:?}, first audible sample after {:?}",
        pressed.elapsed()
    );
    engine.pause();
}
