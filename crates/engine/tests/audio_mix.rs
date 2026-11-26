//! Audio readers and the mixer on the committed FFmpeg fixtures (playback.md §6.3, §6.4, §12):
//! resampling, sample-accurate seeks (with `origin` and audio that starts after the video),
//! volume, split points without a seek and pre-seeked jump cuts.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use app_core::media::MediaPath;
use app_core::model::{ClipId, ClipSource};
use app_core::{Animated, AudioProps, Clip, MediaAsset, MediaId, Project, Time};
use engine::audio::{AudioReader, AudioSources, mix_block, sample_offset};
use engine::media::{fingerprint, probe};

const SR: u32 = 48_000;

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

fn clip(media: MediaId, start: Time, duration: Time, source_in: Time) -> Clip {
    Clip {
        id: ClipId::new(),
        name: "A".into(),
        start,
        duration,
        source: ClipSource::Media { media, source_in },
        link: None,
        visual: None,
        audio: Some(AudioProps::default()),
    }
}

fn project(asset: MediaAsset, clips: Vec<Clip>) -> Project {
    let mut project = Project::new("audio");
    project.media.push(asset);
    project.tracks[1].clips = clips;
    project
}

/// `frames` frames read from `reader` (stereo interleaved).
fn read(reader: &mut AudioReader, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; 2 * frames];
    reader.read(&mut out).expect("read");
    out
}

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

/// 1 kHz at `amplitude`, sample `k` at 48 kHz from media time `start` (seconds).
fn sine(amplitude: f32, start: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|k| {
            let t = start + k as f64 / 48_000.0;
            amplitude * (2.0 * PI * 1000.0 * t).sin() as f32
        })
        .collect()
}

/// Mixes `frames` from `start` in blocks of `block`.
fn mix(project: &Project, sources: &mut AudioSources, start: Time, frames: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(256);
        let mut buf = vec![0.0; 2 * n];
        mix_block(
            project,
            start + sample_offset(done as i64, SR),
            n,
            SR,
            sources,
            &mut buf,
        );
        out.extend(buf);
        done += n;
    }
    out
}

#[test]
fn a_48k_stereo_source_reads_exactly_and_seeks_sample_accurately() {
    let path = fixture("sine-1k-48k.flac");
    let asset = asset_at(&path);
    let stream = asset.info.audio[0].stream_index;
    let mut reader = AudioReader::open(&path, stream, asset.info.origin, SR).unwrap();
    let all = read(&mut reader, 40_000);
    assert!(max_diff(&left(&all), &sine(1.0, 0.0, 40_000)) < 1e-4);
    assert_eq!(reader.seeks(), 0);

    // Seek to an odd sample: the samples equal the continuous decode.
    for target in [12_345usize, 30_001, 7] {
        reader.seek(sample_offset(target as i64, SR)).unwrap();
        let after = read(&mut reader, 2_000);
        assert_eq!(
            &after[..],
            &all[2 * target..2 * (target + 2_000)],
            "seek to {target}"
        );
        assert_eq!(
            reader.position(),
            sample_offset((target + 2_000) as i64, SR)
        );
    }
    // Past the end: silence.
    reader.seek(Time::from_seconds(5)).unwrap();
    assert!(read(&mut reader, 100).iter().all(|&s| s == 0.0));
}

#[test]
fn a_44k_mono_source_is_resampled_in_phase_on_both_channels() {
    let path = fixture("sine-1k-44k-mono.flac");
    let asset = asset_at(&path);
    let mut reader = AudioReader::open(&path, 0, asset.info.origin, SR).unwrap();
    let all = read(&mut reader, 40_000);
    let (l, r): (Vec<f32>, Vec<f32>) = all.chunks(2).map(|f| (f[0], f[1])).unzip();
    assert_eq!(l, r, "mono plays on both channels");
    // In phase with the timeline from the first sample: the resampler adds no delay.
    let error = max_diff(&l[..39_000], &sine(0.5, 0.0, 39_000));
    assert!(error < 0.01, "resampled sine off by {error}");
    assert!(
        l.iter().fold(0.0f32, |m, s| m.max(s.abs())) < 0.51,
        "mono at full level, not -3 dB"
    );

    // A seek lands in phase too.
    reader.seek(sample_offset(24_000, SR)).unwrap();
    let after = left(&read(&mut reader, 4_000));
    let error = max_diff(&after, &sine(0.5, 0.5, 4_000));
    assert!(error < 0.01, "after a seek: off by {error}");
}

#[test]
fn origin_and_late_audio_are_honoured_on_the_vfr_fixture() {
    let path = fixture("vfr-audio-offset.ts");
    let asset = asset_at(&path);
    assert!(asset.info.origin > Time::ZERO, "MPEG-TS starts at 1.4 s");
    let audio = asset.info.audio[0].clone();
    let mut reader = AudioReader::open(&path, audio.stream_index, asset.info.origin, SR).unwrap();
    let all = left(&read(&mut reader, 48_000));
    // The audio starts about 0.25 s after the video: silence before.
    let onset = all.iter().position(|s| s.abs() > 0.05).expect("sound");
    assert!((11_000..12_500).contains(&onset), "onset at sample {onset}");
    assert!(all[..onset - 200].iter().all(|s| s.abs() < 1e-3));

    // Sample-accurate seeks after the onset match the continuous decode (MP2, decoded with
    // preroll), and seeks before it give the silence first.
    for target in [20_017usize, 30_000, 5_000] {
        reader.seek(sample_offset(target as i64, SR)).unwrap();
        let after = left(&read(&mut reader, 4_000));
        let error = max_diff(&after, &all[target..target + 4_000]);
        assert!(error < 2e-3, "seek to {target}: off by {error}");
    }
}

#[test]
fn volume_half_peaks_at_half() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    let mut c = clip(
        asset.id,
        Time::ZERO,
        Time(app_core::FLICKS_PER_SECOND / 2),
        Time::ZERO,
    );
    c.audio.as_mut().unwrap().volume = Animated::constant(0.5);
    let project = project(asset, vec![c]);
    let mut sources = AudioSources::new();
    let out = mix(&project, &mut sources, Time::ZERO, 24_000);
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.5).abs() < 1e-3, "peak {peak}");
    assert!(max_diff(&left(&out), &sine(0.5, 0.0, 24_000)) < 1e-4);
}

#[test]
fn a_split_point_plays_without_a_seek() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    let id = asset.id;
    let half = Time(app_core::FLICKS_PER_SECOND / 2);
    let split = vec![
        clip(id, Time::ZERO, half, Time::ZERO),
        clip(id, half, half, half),
    ];
    let whole = vec![clip(id, Time::ZERO, Time::from_seconds(1), Time::ZERO)];
    let mut sources = AudioSources::new();
    let a = mix(
        &project(asset.clone(), split),
        &mut sources,
        Time::ZERO,
        47_000,
    );
    assert_eq!(sources.seeks(), 0, "the reader continued across the split");
    assert_eq!(sources.reader_count(), 1);
    let b = mix(
        &project(asset, whole),
        &mut AudioSources::new(),
        Time::ZERO,
        47_000,
    );
    assert_eq!(a, b, "the split is inaudible");
}

#[test]
fn a_jump_cut_is_primed_before_it_starts() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    let id = asset.id;
    let half = Time(app_core::FLICKS_PER_SECOND / 2);
    let tenth = Time(app_core::FLICKS_PER_SECOND / 10);
    let project = project(
        asset,
        vec![
            clip(id, Time::ZERO, half, Time::ZERO),
            clip(id, half, half - tenth, tenth),
        ],
    );
    let mut sources = AudioSources::new();
    // Up to the cut, pre-seeking every 100 ms as the feeder does.
    let mut out = Vec::new();
    for step in 0..5 {
        let start = Time(app_core::FLICKS_PER_SECOND / 10 * step);
        sources.prefetch(
            &project,
            start,
            Time(app_core::FLICKS_PER_SECOND * 3 / 2),
            SR,
        );
        out.extend(mix(&project, &mut sources, start, 4_800));
    }
    let seeks = sources.seeks();
    assert_eq!(seeks, 1, "one pre-seek for the jump cut");
    out.extend(mix(&project, &mut sources, half, 4_800));
    assert_eq!(sources.seeks(), seeks, "no seek when the cut plays");
    assert_eq!(sources.reader_count(), 2, "A/B readers");
    let after_cut = left(&out[2 * 24_000..]);
    assert!(max_diff(&after_cut, &sine(1.0, 0.1, 4_800)) < 1e-4);
}

#[test]
fn offline_media_is_a_silent_voice() {
    let mut asset = asset_at(&fixture("sine-1k-48k.flac"));
    asset.path = MediaPath::absolute("C:/no/such/folder/sine.flac");
    let c = clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
    let project = project(asset, vec![c]);
    let mut sources = AudioSources::new();
    let out = mix(&project, &mut sources, Time::ZERO, 2_048);
    assert!(out.iter().all(|&s| s == 0.0));
    assert_eq!(sources.reader_count(), 0);
}
