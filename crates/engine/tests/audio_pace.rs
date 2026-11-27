//! Audio pace per voice on the committed FFmpeg fixtures (clip-speed-audio.md §5.3, §9): the
//! time stage keeps or shifts the pitch and scales the duration, reversed voices play the
//! samples backwards, bursts land where the clip mapping puts them (sync), export and device
//! rates agree (parity), voices above 4x are silent, seeks inside a paced clip land on time,
//! split points continue the voice and jump cuts are prepared ahead.
//!
//! `cargo test -p engine --test audio_pace -- --ignored --nocapture` prints the CPU cost of one
//! stage per voice (design §8).

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::time::Instant;

use app_core::media::MediaPath;
use app_core::model::{ClipId, ClipSource};
use app_core::{AudioProps, Clip, FLICKS_PER_SECOND, MediaAsset, MediaId, Project, Speed, Time};
use engine::audio::{AudioReader, AudioSources, TimeStage, mix_block, sample_offset};
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

fn seconds(s: f64) -> Time {
    Time((s * FLICKS_PER_SECOND as f64).round() as i64)
}

/// A clip at `start` playing `source_in..source_in + span` of `media` at `percent`, backwards
/// if `reverse`; its duration is the longest that fits the span.
fn paced_clip(
    media: MediaId,
    start: Time,
    source_in: Time,
    span: Time,
    percent: u16,
    reverse: bool,
    preserve_pitch: bool,
) -> Clip {
    let speed = Speed::new(percent).unwrap();
    Clip {
        id: ClipId::new(),
        name: "A".into(),
        start,
        duration: speed.timeline_floor(span),
        source: ClipSource::Media {
            media,
            source_in,
            speed,
            reverse,
        },
        link: None,
        visual: None,
        audio: Some(AudioProps {
            preserve_pitch,
            ..AudioProps::default()
        }),
    }
}

fn project(asset: MediaAsset, clips: Vec<Clip>) -> Project {
    let mut project = Project::new("pace");
    project.media.push(asset);
    project.tracks[1].clips = clips;
    project
}

/// Mixes `frames` from `start` at `rate` in blocks of `block` (stereo interleaved).
fn mix_at(
    project: &Project,
    sources: &mut AudioSources,
    start: Time,
    frames: usize,
    rate: u32,
    block: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(2 * frames);
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(block);
        let mut buf = vec![0.0; 2 * n];
        mix_block(
            project,
            start + sample_offset(done as i64, rate),
            n,
            rate,
            sources,
            &mut buf,
        );
        out.extend(buf);
        done += n;
    }
    out
}

/// The device path: 48 kHz in blocks of 256.
fn mix(project: &Project, sources: &mut AudioSources, start: Time, frames: usize) -> Vec<f32> {
    mix_at(project, sources, start, frames, SR, 256)
}

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

fn frames_of(s: f64, rate: u32) -> usize {
    (s * f64::from(rate)).round() as usize
}

/// The frequency of `signal` from its upward zero crossings (interpolated).
fn frequency(signal: &[f32], rate: u32) -> f64 {
    let mut crossings = Vec::new();
    for i in 1..signal.len() {
        let (a, b) = (signal[i - 1], signal[i]);
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f64 + f64::from(a / (a - b)));
        }
    }
    assert!(crossings.len() > 2, "no tone");
    let span = crossings.last().unwrap() - crossings[0];
    (crossings.len() - 1) as f64 / span * f64::from(rate)
}

fn rms(signal: &[f32]) -> f64 {
    (signal.iter().map(|&s| f64::from(s * s)).sum::<f64>() / signal.len() as f64).sqrt()
}

/// The energy centre (seconds) of the burst near `expected`, searched within +-`half` seconds.
fn burst_centre(signal: &[f32], rate: u32, expected: f64, half: f64) -> f64 {
    let lo = frames_of((expected - half).max(0.0), rate);
    let hi = frames_of(expected + half, rate).min(signal.len());
    let (mut energy, mut moment) = (0.0f64, 0.0f64);
    for (i, &v) in signal.iter().enumerate().take(hi).skip(lo) {
        let e = f64::from(v * v);
        energy += e;
        moment += e * i as f64;
    }
    assert!(energy > 0.0, "no burst near {expected} s");
    moment / energy / f64::from(rate)
}

/// The bursts of `bursts-48k.flac`: 40 ms Hann-shaped 1 kHz, centred every 0.5 s from 0.5 s.
fn burst_times() -> impl Iterator<Item = f64> {
    (1..12).map(|k| k as f64 * 0.5)
}

#[test]
fn the_stage_keeps_or_shifts_the_pitch_and_scales_the_duration() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    for (percent, pitch, expected) in [
        (200, true, 1_000.0),
        (200, false, 2_000.0),
        (50, true, 1_000.0),
        (50, false, 500.0),
        (150, true, 1_000.0),
    ] {
        let clip = paced_clip(
            asset.id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(1),
            percent,
            false,
            pitch,
        );
        let duration = clip.duration.flicks() as f64 / FLICKS_PER_SECOND as f64;
        assert!((duration - 100.0 / f64::from(percent)).abs() < 1e-6);
        let project = project(asset.clone(), vec![clip]);
        let mut sources = AudioSources::new();
        let out = left(&mix(
            &project,
            &mut sources,
            Time::ZERO,
            frames_of(duration + 0.25, SR),
        ));
        let tone = &out[frames_of(0.1, SR)..frames_of(duration - 0.1, SR)];
        let f = frequency(tone, SR);
        println!("{percent}% pitch {pitch}: {f:.1} Hz, {duration} s");
        assert!(
            (f - expected).abs() < expected * 0.01,
            "{percent}% pitch {pitch}: {f} Hz, want {expected}"
        );
        // It sounds up to the clip end and not after: the audio lasts duration = 1 s / speed.
        let tail = &out[frames_of(duration - 0.15, SR)..frames_of(duration - 0.05, SR)];
        assert!(rms(tail) > 0.3, "{percent}%: sounds until the end");
        let end = frames_of(duration, SR);
        assert!(
            out[end + 1..].iter().all(|&s| s == 0.0),
            "{percent}%: silent after"
        );
        assert_eq!(sources.paced_voice_count(), 1);
    }
}

#[test]
fn a_reversed_voice_at_1x_plays_the_samples_backwards_exactly() {
    let path = fixture("ramp-48k.flac");
    let asset = asset_at(&path);
    let mut reader = AudioReader::open(&path, 0, asset.info.origin, SR).unwrap();
    let mut forward = vec![0.0; 2 * 96_000];
    reader.read(&mut forward).unwrap();
    // Source 0.25..1.75 s, backwards: crosses a 1 s window boundary.
    let clip = paced_clip(
        asset.id,
        Time::ZERO,
        seconds(0.25),
        seconds(1.5),
        100,
        true,
        true,
    );
    let project = project(asset, vec![clip]);
    let mut sources = AudioSources::new();
    let out = mix(&project, &mut sources, Time::ZERO, 72_000);
    let top = 84_000;
    for j in 0..72_000 {
        let want = [forward[2 * (top - 1 - j)], forward[2 * (top - 1 - j) + 1]];
        assert_eq!([out[2 * j], out[2 * j + 1]], want, "frame {j}");
    }
    // One seek per 1 s window.
    assert_eq!(sources.seeks(), 2);
}

#[test]
fn a_reversed_voice_with_a_stage_descends() {
    let asset = asset_at(&fixture("ramp-48k.flac"));
    for pitch in [true, false] {
        // The whole 2 s ramp (-0.8 to 0.8) backwards at 2x: 1 s from 0.8 down to -0.8.
        let clip = paced_clip(
            asset.id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(2),
            200,
            true,
            pitch,
        );
        let project = project(asset.clone(), vec![clip]);
        let out = left(&mix(&project, &mut AudioSources::new(), Time::ZERO, 48_000));
        let means: Vec<f64> = out
            .chunks(2_400)
            .map(|c| c.iter().map(|&s| f64::from(s)).sum::<f64>() / c.len() as f64)
            .collect();
        assert!(
            means[1] > 0.6 && means[18] < -0.6,
            "pitch {pitch}: {means:?}"
        );
        assert!(
            means[1..19].windows(2).all(|w| w[1] < w[0]),
            "pitch {pitch}: descending {means:?}"
        );
    }
}

/// Where the bursts of `clip` (from `bursts-48k.flac`, starting at 0) land, minus where the clip
/// mapping puts them, in milliseconds.
fn burst_errors(clip: &Clip, out: &[f32], rate: u32) -> Vec<f64> {
    let p = f64::from(clip.speed().percent()) / 100.0;
    let span = 6.0;
    let duration = clip.duration.flicks() as f64 / FLICKS_PER_SECOND as f64;
    burst_times()
        .map(|c| {
            if clip.reversed() {
                (span - c) / p
            } else {
                c / p
            }
        })
        .filter(|&t| t > 0.06 && t < duration - 0.06)
        .map(|t| (burst_centre(out, rate, t, 0.2 / p) - t) * 1_000.0)
        .collect()
}

#[test]
fn bursts_land_where_the_clip_mapping_puts_them() {
    let asset = asset_at(&fixture("bursts-48k.flac"));
    for (percent, pitch, reverse) in [
        (100, true, true),
        (200, true, false),
        (200, false, false),
        (50, true, false),
        (50, false, false),
        (150, true, false),
        (75, true, false),
        (400, true, false),
        (200, true, true),
        (50, false, true),
    ] {
        let clip = paced_clip(
            asset.id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(6),
            percent,
            reverse,
            pitch,
        );
        let duration = clip.duration.flicks() as f64 / FLICKS_PER_SECOND as f64;
        let project = project(asset.clone(), vec![clip.clone()]);
        let out = left(&mix(
            &project,
            &mut AudioSources::new(),
            Time::ZERO,
            frames_of(duration, SR),
        ));
        let errors = burst_errors(&clip, &out, SR);
        let worst = errors.iter().fold(0.0f64, |m, e| m.max(e.abs()));
        println!("{percent}% pitch {pitch} reverse {reverse}: worst {worst:.2} ms {errors:.2?}");
        assert!(errors.len() >= 5);
        assert!(
            worst <= 5.0,
            "{percent}% pitch {pitch} reverse {reverse}: {errors:?}"
        );
    }
}

/// 10 minutes of output at 1.5x (15 minutes of source) through one stage: the bursts at the
/// end are still within 10 ms of the mapping and the backlog never leaves its bounds (the
/// voice would re-anchor).
#[test]
fn ten_minutes_at_1_5x_stay_in_sync() {
    let sr = SR as usize;
    let mut stage = TimeStage::new(Speed::new(150).unwrap(), true, SR).unwrap();
    let burst = sr / 25;
    let every = 10 * sr;
    let preroll = stage.preroll_frames() as i64;
    // Source frame `i` (from the wanted start): a burst centred on every multiple of 10 s.
    let sample = |i: i64| -> f32 {
        if i < 0 {
            return 0.0;
        }
        let d = (i as usize + burst / 2) % every;
        if d >= burst {
            return 0.0;
        }
        let envelope = 0.5 - 0.5 * (2.0 * PI * d as f64 / burst as f64).cos();
        (envelope * (2.0 * PI * 1000.0 * d as f64 / sr as f64).sin()) as f32
    };
    let mut position = -preroll;
    let mut feed = |chunk: &mut [f32]| {
        for frame in chunk.as_chunks_mut::<2>().0 {
            let v = sample(position);
            *frame = [v, v];
            position += 1;
        }
        Ok(())
    };
    let total = 600 * sr;
    let keep_from = total - 30 * sr;
    let mut tail = Vec::with_capacity(2 * (total - keep_from));
    let mut chunk = vec![0.0f32; 2 * 1_024];
    let slack = sr / 100;
    let (mut low, mut high) = (i64::MAX, i64::MIN);
    let mut done = 0;
    while done < total {
        stage.take(&mut chunk, &mut feed).unwrap();
        let backlog = stage.backlog_frames();
        low = low.min(backlog);
        high = high.max(backlog);
        if done >= keep_from {
            tail.extend(chunk.iter().step_by(2));
        }
        done += 1_024;
    }
    println!(
        "backlog {low}..={high} frames (limit -{slack}..={})",
        stage.backlog_limit(slack)
    );
    assert!(low >= -(slack as i64) && high <= stage.backlog_limit(slack));
    let offset = keep_from as f64 / SR as f64;
    let mut errors = Vec::new();
    for k in 0..3 {
        // Bursts at source 870, 880, 890 s: timeline 580, 586.67, 593.33 s.
        let t = (870.0 + 10.0 * k as f64) / 1.5;
        let centre = burst_centre(&tail, SR, t - offset, 0.5) + offset;
        errors.push((centre - t) * 1_000.0);
    }
    println!("after 10 minutes at 1.5x: {errors:.2?} ms");
    assert!(errors.iter().all(|e| e.abs() <= 10.0), "{errors:?}");
}

#[test]
fn export_and_device_rates_agree() {
    let asset = asset_at(&fixture("bursts-48k.flac"));
    for (percent, pitch, reverse, tolerance_ms) in [
        // Plain resampling and reversed windows are sample-exact on both rates: within one
        // sample period of 48 kHz.
        (200, false, false, 1_000.0 / 48_000.0),
        (100, true, true, 1_000.0 / 48_000.0),
        // atempo's windows differ with the rate (2048 vs 4096 frames): within its jitter.
        (200, true, false, 5.0),
        (50, true, false, 5.0),
    ] {
        let clip = paced_clip(
            asset.id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(6),
            percent,
            reverse,
            pitch,
        );
        let duration = clip.duration.flicks() as f64 / FLICKS_PER_SECOND as f64;
        let project = project(asset.clone(), vec![clip.clone()]);
        // Export: the project rate in blocks of 1024; device: 96 kHz in blocks of 256.
        let export = left(&mix_at(
            &project,
            &mut AudioSources::new(),
            Time::ZERO,
            frames_of(duration, 48_000),
            48_000,
            1_024,
        ));
        let device = left(&mix_at(
            &project,
            &mut AudioSources::new(),
            Time::ZERO,
            frames_of(duration, 96_000),
            96_000,
            256,
        ));
        let a = burst_errors(&clip, &export, 48_000);
        let b = burst_errors(&clip, &device, 96_000);
        let worst = a
            .iter()
            .zip(&b)
            .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
        println!("{percent}% pitch {pitch} reverse {reverse}: 48k vs 96k worst {worst:.4} ms");
        assert!(worst <= tolerance_ms + 1e-3, "{percent}%: {a:?} vs {b:?}");
        assert!(b.iter().all(|e| e.abs() <= 5.0), "96 kHz: {b:?}");
    }
}

#[test]
fn voices_faster_than_4x_are_silent_and_open_no_reader() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    let clip = paced_clip(
        asset.id,
        Time::ZERO,
        Time::ZERO,
        Time::from_seconds(1),
        500,
        false,
        true,
    );
    let project = project(asset, vec![clip]);
    let mut sources = AudioSources::new();
    sources.prefetch(&project, Time(-1), Time::from_seconds(1), SR);
    let out = mix(&project, &mut sources, Time::ZERO, 9_600);
    assert!(out.iter().all(|&s| s == 0.0));
    assert_eq!(sources.reader_count(), 0);
}

#[test]
fn a_seek_inside_a_paced_clip_lands_on_time() {
    let asset = asset_at(&fixture("bursts-48k.flac"));
    // Reversed at 1x and plain resampling at 2x are sample-accurate: the samples after the seek
    // equal the continuous mix.
    for (percent, reverse, pitch, tolerance) in
        [(100, true, true, 0.0f32), (200, false, false, 2e-3)]
    {
        let clip = paced_clip(
            asset.id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(6),
            percent,
            reverse,
            pitch,
        );
        let project = project(asset.clone(), vec![clip]);
        let whole = mix(&project, &mut AudioSources::new(), Time::ZERO, 96_000);
        let at = 33_333;
        let mut sources = AudioSources::new();
        let after = mix(&project, &mut sources, sample_offset(at as i64, SR), 48_000);
        let diff = after
            .iter()
            .zip(&whole[2 * at..])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        println!("{percent}% reverse {reverse}: after a seek off by {diff}");
        assert!(
            diff <= tolerance,
            "{percent}% reverse {reverse}: off by {diff}"
        );
    }
    // atempo: the bursts after a seek land within its 5 ms.
    let clip = paced_clip(
        asset.id,
        Time::ZERO,
        Time::ZERO,
        Time::from_seconds(6),
        200,
        false,
        true,
    );
    let project = project(asset, vec![clip]);
    let from = 1.1;
    let out = left(&mix(
        &project,
        &mut AudioSources::new(),
        seconds(from),
        frames_of(1.5, SR),
    ));
    for c in [2.5, 3.0, 3.5, 4.0, 5.0] {
        let t = c / 2.0 - from;
        let error = (burst_centre(&out, SR, t, 0.1) - t) * 1_000.0;
        assert!(
            error.abs() <= 5.0,
            "burst at {c} s after a seek: {error} ms"
        );
    }
}

#[test]
fn a_split_point_of_a_paced_clip_continues_its_voice() {
    let asset = asset_at(&fixture("bursts-48k.flac"));
    let id = asset.id;
    for (percent, reverse) in [(200, false), (150, true), (100, true)] {
        let whole = paced_clip(
            id,
            Time::ZERO,
            Time::ZERO,
            Time::from_seconds(4),
            percent,
            reverse,
            true,
        );
        // Split at 0.7 s (timeline), the way SplitClip keeps every content moment.
        let at = seconds(0.7);
        let mut a = whole.clone();
        a.id = ClipId::new();
        a.duration = at;
        let mut b = whole.clone();
        b.id = ClipId::new();
        b.start = at;
        b.duration = whole.duration - at;
        let (a_in, b_in) = (
            whole.sub_source_in(Time::ZERO, a.duration),
            whole.sub_source_in(at, b.duration),
        );
        for (clip, source_in) in [(&mut a, a_in), (&mut b, b_in)] {
            let ClipSource::Media { source_in: s, .. } = &mut clip.source else {
                unreachable!()
            };
            *s = source_in;
        }
        let frames = 48_000;
        let mut split_sources = AudioSources::new();
        let split = mix(
            &project(asset.clone(), vec![a, b]),
            &mut split_sources,
            Time::ZERO,
            frames,
        );
        let mut whole_sources = AudioSources::new();
        let one = mix(
            &project(asset.clone(), vec![whole]),
            &mut whole_sources,
            Time::ZERO,
            frames,
        );
        assert_eq!(
            split_sources.seeks(),
            whole_sources.seeks(),
            "{percent}% reverse {reverse}: no re-anchor at the split"
        );
        assert_eq!(split_sources.reader_count(), 1);
        assert_eq!(
            split, one,
            "{percent}% reverse {reverse}: the split is inaudible"
        );
    }
}

#[test]
fn a_paced_jump_cut_is_prepared_ahead() {
    let asset = asset_at(&fixture("bursts-48k.flac"));
    let id = asset.id;
    let half = seconds(0.5);
    let first = paced_clip(
        id,
        Time::ZERO,
        Time::ZERO,
        Time::from_seconds(1),
        200,
        false,
        true,
    );
    let second = paced_clip(
        id,
        half,
        seconds(3.0),
        Time::from_seconds(2),
        200,
        true,
        true,
    );
    let project = project(asset, vec![first, second]);
    let mut sources = AudioSources::new();
    let mut out = Vec::new();
    for step in 0..5 {
        let start = Time(FLICKS_PER_SECOND / 10 * step);
        sources.prefetch(&project, start, seconds(1.5), SR);
        out.extend(mix(&project, &mut sources, start, 4_800));
    }
    let seeks = sources.seeks();
    assert_eq!(
        sources.paced_voice_count(),
        2,
        "the cut's voice is built ahead"
    );
    out.extend(mix(&project, &mut sources, half, 48_000));
    assert_eq!(
        sources.seeks(),
        seeks + 2,
        "only the two reverse windows below the primed one seek"
    );
    // The reversed 2x clip plays source 5..3 s: bursts at 4.5, 4.0, 3.5 s land at
    // 0.5 + (5 - c) / 2.
    let out = left(&out);
    for c in [4.5, 4.0, 3.5] {
        let t = 0.5 + (5.0 - c) / 2.0;
        let error = (burst_centre(&out, SR, t, 0.1) - t) * 1_000.0;
        assert!(error.abs() <= 5.0, "burst at {c} s: {error} ms");
    }
}

#[test]
fn a_pace_change_reanchors_the_voice() {
    let asset = asset_at(&fixture("sine-1k-48k.flac"));
    let clip = paced_clip(
        asset.id,
        Time::ZERO,
        Time::ZERO,
        Time::from_seconds(1),
        200,
        false,
        false,
    );
    let mut project = project(asset, vec![clip]);
    let mut sources = AudioSources::new();
    mix(&project, &mut sources, Time::ZERO, 4_800);
    let seeks = sources.seeks();
    // A speed gesture: 2x -> 1.5x without pitch, same clip.
    let c = &mut project.tracks[1].clips[0];
    let ClipSource::Media { speed, .. } = &mut c.source else {
        unreachable!()
    };
    *speed = Speed::new(150).unwrap();
    c.duration = Speed::new(150)
        .unwrap()
        .timeline_floor(Time::from_seconds(1));
    let at = sample_offset(4_800, SR);
    let out = left(&mix(&project, &mut sources, at, 9_600));
    assert_eq!(sources.seeks(), seeks + 1, "re-anchored once");
    let f = frequency(&out[480..], SR);
    assert!((f - 1_500.0).abs() < 15.0, "{f} Hz");
    // Back at 1x: the reader returns to the pool.
    let c = &mut project.tracks[1].clips[0];
    let ClipSource::Media { speed, .. } = &mut c.source else {
        unreachable!()
    };
    *speed = Speed::NORMAL;
    c.duration = Time::from_seconds(1);
    mix(&project, &mut sources, seconds(0.5), 4_800);
    assert_eq!(sources.paced_voice_count(), 0);
    assert_eq!(sources.reader_count(), 1);
}

/// A 1 kHz tone at half scale from source frame `*position` on.
fn tone(position: &mut u64, chunk: &mut [f32]) {
    for frame in chunk.as_chunks_mut::<2>().0 {
        let v = (0.5 * (2.0 * PI * 1000.0 * *position as f64 / 48_000.0).sin()) as f32;
        *frame = [v, v];
        *position += 1;
    }
}

/// CPU per voice (design §8): one stage producing 60 s of output from a 1 kHz tone, on this
/// thread (the tone's own cost subtracted). Prints the share of one core.
#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn stage_cpu_per_voice() {
    let seconds = 60usize;
    for (percent, pitch) in [
        (50, true),
        (150, true),
        (200, true),
        (400, true),
        (200, false),
        (50, false),
    ] {
        let source_frames = seconds * 48_000 * percent as usize / 100;
        let mut position = 0;
        let mut scratch = vec![0.0f32; 2 * 512];
        let started = Instant::now();
        for _ in 0..source_frames / 512 {
            tone(&mut position, &mut scratch);
        }
        let tone_only = started.elapsed().as_secs_f64();

        let started = Instant::now();
        let mut stage = TimeStage::new(Speed::new(percent).unwrap(), pitch, SR).unwrap();
        let built = started.elapsed();
        let mut position = 0;
        let mut feed = |chunk: &mut [f32]| {
            tone(&mut position, chunk);
            Ok(())
        };
        let mut chunk = vec![0.0f32; 2 * 256];
        let started = Instant::now();
        for _ in 0..seconds * 48_000 / 256 {
            stage.take(&mut chunk, &mut feed).unwrap();
        }
        let total = started.elapsed().as_secs_f64() - tone_only;
        println!(
            "{percent:>3}% pitch {pitch}: {:.2} % of one core ({:.2} ms per second of output), \
             first build {built:?}",
            total / seconds as f64 * 100.0,
            total / seconds as f64 * 1_000.0
        );
    }
}
