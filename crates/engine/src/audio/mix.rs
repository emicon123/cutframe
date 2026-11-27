//! The mixer (playback.md §6.3, timeline-editing.md §7): shared by playback (at the device rate,
//! on the feeder thread) and export (at the project rate, on the export thread).
//!
//! Voices are the clips with `audio` props on unmuted tracks (hidden tracks still sound),
//! main-track video clips with audio included. Each voice's PCM comes from a [`PcmProvider`],
//! times `volume.at(local) x fade(local)`, the gain evaluated at the ends of each linear piece
//! and ramped per sample; clip boundaries are sample-accurate. The voices are summed, then
//! clamped to -1..=1. `mix_block` itself never allocates (the per-voice scratch is on the stack).

use app_core::media::MediaAsset;
use app_core::model::{ClipId, ClipSource};
use app_core::{AudioProps, Clip, Project, Time};

use super::{sample_offset, samples_before};

/// `mix_block` asks a provider for at most this many frames per read (export uses blocks of
/// exactly this size, the AAC frame).
pub const MIX_CHUNK_FRAMES: usize = 1024;

/// One voice's read: PCM of `media` from `source_time` on.
#[derive(Debug, Clone, Copy)]
pub struct Voice<'a> {
    pub clip: ClipId,
    pub media: &'a MediaAsset,
    /// Media time of the first frame to read.
    pub source_time: Time,
    pub sample_rate: u32,
}

/// Where the mixer gets PCM from: [`super::AudioSources`] (FFmpeg readers), or a fake in tests.
/// Construct and use it on any one thread (the feeder, the export thread).
pub trait PcmProvider {
    /// A [`mix_block`] call begins (the reads that follow belong to one block).
    fn begin_block(&mut self) {}

    /// Fills `out` (stereo interleaved `f32`, `out.len() / 2` frames) with the voice's PCM at
    /// `voice.sample_rate` from `voice.source_time`. Returns false if the voice is silent
    /// (offline media, a decode error); `out` is ignored then.
    fn read(&mut self, voice: &Voice<'_>, out: &mut [f32]) -> bool;
}

/// Mixes `frames` frames of the timeline from `start` at `sample_rate` into `out` (stereo
/// interleaved, at least `2 x frames` long; the first `2 x frames` values are written). Frame
/// `k` is the timeline time `start + sample_offset(k)`.
pub fn mix_block(
    project: &Project,
    start: Time,
    frames: usize,
    sample_rate: u32,
    sources: &mut dyn PcmProvider,
    out: &mut [f32],
) {
    let out = &mut out[..2 * frames];
    out.fill(0.0);
    sources.begin_block();
    let end = start + sample_offset(frames as i64, sample_rate);
    let mut scratch = [0.0f32; 2 * MIX_CHUNK_FRAMES];
    for track in project.tracks.iter().filter(|t| !t.muted) {
        let first = track.clips.partition_point(|c| c.end() <= start);
        for clip in track.clips[first..].iter().take_while(|c| c.start < end) {
            let (Some(props), ClipSource::Media { media, .. }) = (&clip.audio, &clip.source) else {
                continue;
            };
            let Some(asset) = project.media_asset(*media) else {
                continue;
            };
            if asset.info.audio.is_empty() {
                continue;
            }
            let k0 = clamp_frames(samples_before(clip.start - start, sample_rate), frames);
            let k1 = clamp_frames(samples_before(clip.end() - start, sample_rate), frames);
            let mut a = k0;
            while a < k1 {
                let n = (k1 - a).min(MIX_CHUNK_FRAMES);
                let t = start + sample_offset(a as i64, sample_rate);
                // TODO(0370 step 5): pace per voice (speed, reverse, pitch, silent above 4x).
                // Until then a sped-up clip reads 1x audio from its mapped source time.
                let Some(source_time) = clip.source_time(t) else {
                    break;
                };
                let pcm = &mut scratch[..2 * n];
                let voice = Voice {
                    clip: clip.id,
                    media: asset,
                    source_time,
                    sample_rate,
                };
                if sources.read(&voice, pcm) {
                    let gain = Gain {
                        clip,
                        props,
                        start,
                        sample_rate,
                    };
                    gain.apply(a, a + n, pcm, out);
                }
                a += n;
            }
        }
    }
    for sample in out.iter_mut() {
        *sample = sample.clamp(-1.0, 1.0);
    }
}

fn clamp_frames(k: i64, frames: usize) -> usize {
    usize::try_from(k.max(0)).unwrap_or(usize::MAX).min(frames)
}

/// A voice's gain over a block whose frame 0 is at timeline time `start`.
struct Gain<'a> {
    clip: &'a Clip,
    props: &'a AudioProps,
    start: Time,
    sample_rate: u32,
}

impl Gain<'_> {
    /// Offset from the clip start of block frame `k`.
    fn local(&self, k: usize) -> Time {
        self.start + sample_offset(k as i64, self.sample_rate) - self.clip.start
    }

    /// `volume.at(local) x fade_in x fade_out`, the fades linear.
    fn at(&self, local: Time) -> f32 {
        let fraction = |num: Time, den: Time| -> f32 {
            (num.flicks() as f64 / den.flicks() as f64).clamp(0.0, 1.0) as f32
        };
        let fade_in = if self.props.fade_in > Time::ZERO {
            fraction(local, self.props.fade_in)
        } else {
            1.0
        };
        let fade_out = if self.props.fade_out > Time::ZERO {
            fraction(self.clip.duration - local, self.props.fade_out)
        } else {
            1.0
        };
        self.props.volume.at(local) * fade_in * fade_out
    }

    /// Adds `pcm` (the voice's frames `a..b` of the block) times the gain into `out`. The gain
    /// is linear between the knees of the fades, so it is evaluated at the ends of each piece and
    /// ramped per sample.
    fn apply(&self, a: usize, b: usize, pcm: &[f32], out: &mut [f32]) {
        let knee = |at: Time| {
            let k = samples_before(self.clip.start + at - self.start, self.sample_rate);
            usize::try_from(k.max(0)).unwrap_or(usize::MAX).clamp(a, b)
        };
        let mut cuts = [
            a,
            knee(self.props.fade_in),
            knee(self.clip.duration - self.props.fade_out),
            b,
        ];
        cuts.sort_unstable();
        for piece in cuts.windows(2) {
            let (p, q) = (piece[0], piece[1]);
            if p == q {
                continue;
            }
            let g0 = self.at(self.local(p));
            let g1 = self.at(self.local(q));
            let step = (g1 - g0) / (q - p) as f32;
            for j in p..q {
                let g = g0 + step * (j - p) as f32;
                let i = 2 * (j - a);
                out[2 * j] += pcm[i] * g;
                out[2 * j + 1] += pcm[i + 1] * g;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use app_core::media::{AudioStreamInfo, Fingerprint, MediaInfo, MediaPath};
    use app_core::model::{Track, TrackKind};
    use app_core::{Animated, MediaId, MediaKind};

    use super::*;

    /// A provider of constant PCM (1.0 left, 0.5 right) that records its reads.
    #[derive(Debug, Default)]
    pub(crate) struct Constant {
        pub reads: Vec<(ClipId, Time, usize)>,
        pub silent: Vec<MediaId>,
    }

    impl PcmProvider for Constant {
        fn read(&mut self, voice: &Voice<'_>, out: &mut [f32]) -> bool {
            self.reads
                .push((voice.clip, voice.source_time, out.len() / 2));
            if self.silent.contains(&voice.media.id) {
                return false;
            }
            for frame in out.as_chunks_mut::<2>().0 {
                frame[0] = 1.0;
                frame[1] = 0.5;
            }
            true
        }
    }

    pub(crate) fn audio_asset(seconds: i64) -> MediaAsset {
        MediaAsset {
            id: MediaId::new(),
            kind: MediaKind::Audio,
            path: MediaPath::absolute("C:/does/not/exist.wav"),
            fingerprint: Fingerprint {
                size: 0,
                modified: 0,
                hash: "0".repeat(32),
            },
            info: MediaInfo {
                container: "wav".into(),
                origin: Time::ZERO,
                duration: Some(Time::from_seconds(seconds)),
                video: None,
                audio: vec![AudioStreamInfo {
                    stream_index: 0,
                    codec: "pcm_s16le".into(),
                    sample_rate: 48_000,
                    channels: 2,
                }],
            },
        }
    }

    pub(crate) fn audio_clip(media: MediaId, start: Time, duration: Time, source_in: Time) -> Clip {
        Clip {
            id: ClipId::new(),
            name: "A".into(),
            start,
            duration,
            source: ClipSource::media_at(media, source_in),
            link: None,
            visual: None,
            audio: Some(AudioProps::default()),
        }
    }

    fn project_with(clips: Vec<Clip>, asset: MediaAsset) -> Project {
        let mut project = Project::new("mix");
        project.media.push(asset);
        project.tracks[1].clips = clips;
        project
    }

    const SR: u32 = 48_000;

    fn mix(project: &Project, start: Time, frames: usize, sources: &mut Constant) -> Vec<f32> {
        let mut out = vec![9.0; 2 * frames];
        mix_block(project, start, frames, SR, sources, &mut out);
        out
    }

    #[test]
    fn clip_boundaries_are_sample_accurate() {
        let asset = audio_asset(10);
        // Starts 10.5 samples into the block: the first sample at or after it is frame 11.
        let start = sample_offset(10, SR) + Time(14_700 / 2);
        let clip = audio_clip(
            asset.id,
            start,
            sample_offset(100, SR),
            Time::from_seconds(2),
        );
        let project = project_with(vec![clip.clone()], asset);
        let mut sources = Constant::default();
        let out = mix(&project, Time::ZERO, 256, &mut sources);
        assert_eq!(out[2 * 10], 0.0);
        assert_eq!(out[2 * 11], 1.0);
        assert_eq!(out[2 * 11 + 1], 0.5);
        // The clip lasts exactly 100 samples: frames 11..=110.
        assert_eq!(out[2 * 110], 1.0);
        assert_eq!(out[2 * 111], 0.0);
        // The provider was asked from the media time of frame 11.
        let (id, source_time, frames) = sources.reads[0];
        assert_eq!(id, clip.id);
        assert_eq!(frames, 100);
        assert_eq!(
            source_time,
            Time::from_seconds(2) + sample_offset(11, SR) - start
        );
    }

    #[test]
    fn volume_scales_and_overlapping_voices_sum_then_clamp() {
        let asset = audio_asset(10);
        let mut a = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        a.audio.as_mut().unwrap().volume = Animated::constant(0.5);
        let b = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        let mut project = project_with(vec![a], asset);
        let mut other = Track::new(TrackKind::Audio, "Audio 2");
        other.clips.push(b);
        project.tracks.push(other);
        let out = mix(&project, Time::ZERO, 64, &mut Constant::default());
        // Left: 0.5 + 1.0 clamps to 1; right: 0.25 + 0.5.
        assert!(out.as_chunks::<2>().0.iter().all(|f| *f == [1.0, 0.75]));

        project.tracks[1].clips[0].audio.as_mut().unwrap().volume = Animated::constant(0.25);
        project.tracks[2].clips[0].audio.as_mut().unwrap().volume = Animated::constant(0.25);
        let out = mix(&project, Time::ZERO, 64, &mut Constant::default());
        assert!(out.as_chunks::<2>().0.iter().all(|f| *f == [0.5, 0.25]));
    }

    #[test]
    fn muted_tracks_are_silent_and_hidden_tracks_sound() {
        let asset = audio_asset(10);
        let clip = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        let mut project = project_with(vec![clip], asset);
        project.tracks[1].hidden = true;
        let out = mix(&project, Time::ZERO, 16, &mut Constant::default());
        assert_eq!(&out[..2], &[1.0, 0.5], "hidden still sounds");
        project.tracks[1].muted = true;
        let mut sources = Constant::default();
        let out = mix(&project, Time::ZERO, 16, &mut sources);
        assert!(out.iter().all(|&s| s == 0.0));
        assert!(sources.reads.is_empty(), "a muted voice is not even read");
    }

    #[test]
    fn main_track_video_clips_with_audio_sound_and_clips_without_do_not() {
        let asset = audio_asset(10);
        let mut with = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        with.visual = Some(app_core::VisualProps::default());
        let mut without = with.clone();
        without.id = ClipId::new();
        without.start = Time::from_seconds(1);
        without.audio = None;
        let mut project = project_with(Vec::new(), asset);
        project.tracks[0].clips = vec![with, without];
        let out = mix(&project, Time::ZERO, 8, &mut Constant::default());
        assert_eq!(&out[..2], &[1.0, 0.5]);
        let out = mix(&project, Time::from_seconds(1), 8, &mut Constant::default());
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn fades_are_linear() {
        let asset = audio_asset(10);
        let mut clip = audio_clip(asset.id, Time::ZERO, sample_offset(1_000, SR), Time::ZERO);
        let props = clip.audio.as_mut().unwrap();
        props.fade_in = sample_offset(100, SR);
        props.fade_out = sample_offset(200, SR);
        let project = project_with(vec![clip], asset);
        // Mixed in blocks of 256 that do not line up with the knees.
        let mut left = Vec::new();
        for block in 0..4 {
            let out = mix(
                &project,
                sample_offset(block * 256, SR),
                256,
                &mut Constant::default(),
            );
            left.extend(out.as_chunks::<2>().0.iter().map(|f| f[0]));
        }
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        for (k, &v) in left.iter().enumerate().take(1_000) {
            let expected = if k < 100 {
                k as f32 / 100.0
            } else if k >= 800 {
                (1_000 - k) as f32 / 200.0
            } else {
                1.0
            };
            assert!(close(v, expected), "sample {k}: {v} != {expected}");
        }
        assert!(left[1_000..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn silent_voices_add_nothing() {
        let asset = audio_asset(10);
        let clip = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        let mut sources = Constant {
            silent: vec![asset.id],
            ..Constant::default()
        };
        let project = project_with(vec![clip], asset);
        let out = mix(&project, Time::ZERO, 32, &mut sources);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn long_blocks_are_read_in_chunks() {
        let asset = audio_asset(10);
        let clip = audio_clip(asset.id, Time::ZERO, Time::from_seconds(1), Time::ZERO);
        let project = project_with(vec![clip], asset);
        let mut sources = Constant::default();
        mix(&project, Time::ZERO, 3_000, &mut sources);
        let frames: Vec<usize> = sources.reads.iter().map(|r| r.2).collect();
        assert_eq!(frames, [1_024, 1_024, 952]);
        assert_eq!(sources.reads[1].1, sample_offset(1_024, SR));
    }
}
