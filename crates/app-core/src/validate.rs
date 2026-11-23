//! Invariants of a [`Project`], checked when a file is loaded (ADR-0002 §3) and when settings change.
//!
//! An invalid project is reported, never repaired: every command may assume these invariants hold.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::media::{MediaId, MediaKind, MediaValidationError, validate_media};
use crate::model::{
    CanvasBackground, Clip, ClipId, ClipSource, LinkId, Project, ProjectSettings, Track, TrackId,
    TrackKind,
};
use crate::props::PropertyName;
use crate::time::{MAX_IPC_FLICKS, Time};

/// Canvas width and height limits, in pixels. Both must also be even (yuv420 encoders need it).
pub const CANVAS_SIZE_RANGE: std::ops::RangeInclusive<u32> = 16..=8192;
/// Highest supported frame rate, in frames per second. The lowest is 1.
pub const MAX_FPS: u32 = 240;
/// Supported audio sample rates, in Hz.
pub const SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    /// Project, track and clip ids share one id space.
    #[error("id {0} is used more than once")]
    DuplicateId(Uuid),
    #[error("project name is empty")]
    EmptyName,
    #[error("clip {clip:?} has a duration that is not positive")]
    ClipNotPositive { clip: ClipId },
    #[error("clip {clip:?} starts before zero")]
    ClipNegativeStart { clip: ClipId },
    #[error("clip {clip:?} ends after the largest supported time")]
    TimeOutOfRange { clip: ClipId },
    #[error("clip {clip:?} on track {track:?} starts before the clip before it")]
    ClipsOutOfOrder { track: TrackId, clip: ClipId },
    #[error("clips {first:?} and {second:?} on track {track:?} overlap")]
    ClipsOverlap {
        track: TrackId,
        first: ClipId,
        second: ClipId,
    },
    #[error("canvas size {width}x{height} is not even or outside 16..=8192")]
    CanvasSize { width: u32, height: u32 },
    #[error("frame rate {num}/{den} is outside 1..=240 fps")]
    FrameRateRange { num: u32, den: u32 },
    #[error("sample rate {0} Hz is not supported")]
    SampleRate(u32),
    /// A media asset breaks a media invariant (design `media-import.md` §3).
    #[error(transparent)]
    Media(#[from] MediaValidationError),
    #[error("main track {0:?} is not a video track of the project")]
    MainTrack(TrackId),
    /// Tracks must be overlay (video, text) tracks, then the main track, then audio tracks.
    #[error("track {0:?} is in the wrong section")]
    TrackSection(TrackId),
    /// The clip's source, or its visual/audio props, cannot live on its track (§3.3).
    #[error("clip {clip:?} does not fit its {kind:?} track")]
    ClipTrackMismatch { clip: ClipId, kind: TrackKind },
    #[error("clip {clip:?} references media {media} that is not in the project")]
    ClipMediaMissing { clip: ClipId, media: MediaId },
    /// The media's kind or audio does not match the clip (e.g. audio props without audio).
    #[error("clip {clip:?} does not match the type of media {media}")]
    ClipMediaMismatch { clip: ClipId, media: MediaId },
    /// `source_in` is negative, or the used range runs past the media's end (images: not 0).
    #[error("clip {clip:?} uses a source range outside its media")]
    SourceRange { clip: ClipId },
    #[error("the magnetic main track has a gap or overlap before clip {clip:?}")]
    MainTrackNotContiguous { clip: ClipId },
    #[error("link {0:?} has fewer than two clips")]
    LinkTooSmall(LinkId),
    #[error("link {0:?} has two clips on one track")]
    LinkSameTrack(LinkId),
    #[error("clip {clip:?} has property {property} out of range")]
    PropertyRange {
        clip: ClipId,
        property: PropertyName,
    },
    /// A blur amount outside 0..=1, or an image background that is not an image asset.
    #[error("the canvas background is invalid")]
    Background,
}

impl ProjectSettings {
    /// Checks the ranges every project must satisfy. Used by load, new project and
    /// [`crate::command::SetProjectSettings`]. Flick alignment of the frame rate is already
    /// guaranteed by [`crate::FrameRate`].
    pub fn validate(&self) -> Result<(), ValidationError> {
        let size_ok = |v: u32| v.is_multiple_of(2) && CANVAS_SIZE_RANGE.contains(&v);
        if !size_ok(self.width) || !size_ok(self.height) {
            return Err(ValidationError::CanvasSize {
                width: self.width,
                height: self.height,
            });
        }
        let (num, den) = (self.frame_rate.num(), self.frame_rate.den());
        // 1 <= num/den <= MAX_FPS, in integers (u64: `MAX_FPS * den` overflows u32).
        if num < den || u64::from(num) > u64::from(MAX_FPS) * u64::from(den) {
            return Err(ValidationError::FrameRateRange { num, den });
        }
        if !SAMPLE_RATES.contains(&self.sample_rate) {
            return Err(ValidationError::SampleRate(self.sample_rate));
        }
        Ok(())
    }
}

impl Project {
    /// Checks every invariant (ADR-0002 §3, design `timeline-editing.md` §3.4): settings ranges, a
    /// non-blank name, unique ids, clips that have a positive duration, lie within
    /// `0..=MAX_IPC_FLICKS` and are sorted and non-overlapping on each track, the track sections
    /// around the main track, clip sources and props that fit their track and media, a contiguous
    /// main track while magnetic, link groups and the canvas background. Clip times need not lie on
    /// the frame grid, so a frame-rate change never makes a project invalid. Media assets are
    /// checked by [`validate_media`]; their ids are unique among media (uuid v7 makes a clash with
    /// a track or clip id practically impossible).
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName);
        }
        self.settings.validate()?;
        validate_media(&self.media)?;

        let mut ids = HashSet::new();
        let mut unique = |id: Uuid| {
            if ids.insert(id) {
                Ok(())
            } else {
                Err(ValidationError::DuplicateId(id))
            }
        };
        unique(self.id.0)?;
        for track in &self.tracks {
            unique(track.id.0)?;
            for clip in &track.clips {
                unique(clip.id.0)?;
            }
        }

        self.validate_sections()?;
        let media: HashMap<MediaId, &crate::media::MediaAsset> =
            self.media.iter().map(|m| (m.id, m)).collect();
        for track in &self.tracks {
            validate_clip_times(track)?;
            for clip in &track.clips {
                validate_clip(track.kind, clip, &media)?;
            }
        }
        if self.magnetic
            && let Some(main) = self.main()
        {
            let mut end = Time::ZERO;
            for clip in &main.clips {
                if clip.start != end {
                    return Err(ValidationError::MainTrackNotContiguous { clip: clip.id });
                }
                end = clip.end();
            }
        }
        self.validate_links()?;
        let background_ok = match self.background {
            CanvasBackground::Color { .. } => true,
            CanvasBackground::Blur { amount } => (0.0..=1.0).contains(&amount),
            CanvasBackground::Image { media: id } => {
                media.get(&id).is_some_and(|m| m.kind == MediaKind::Image)
            }
        };
        if !background_ok {
            return Err(ValidationError::Background);
        }
        Ok(())
    }

    /// Overlay tracks (video, text), then the main video track, then audio tracks.
    fn validate_sections(&self) -> Result<(), ValidationError> {
        let main = self
            .main_index()
            .filter(|&i| self.tracks[i].kind == TrackKind::Video)
            .ok_or(ValidationError::MainTrack(self.main_track))?;
        for (index, track) in self.tracks.iter().enumerate() {
            let fits = match index.cmp(&main) {
                std::cmp::Ordering::Less => track.kind != TrackKind::Audio,
                std::cmp::Ordering::Equal => true,
                std::cmp::Ordering::Greater => track.kind == TrackKind::Audio,
            };
            if !fits {
                return Err(ValidationError::TrackSection(track.id));
            }
        }
        Ok(())
    }

    /// Every link is used by at least two clips, on pairwise different tracks.
    fn validate_links(&self) -> Result<(), ValidationError> {
        let mut groups: HashMap<LinkId, Vec<TrackId>> = HashMap::new();
        for track in &self.tracks {
            for link in track.clips.iter().filter_map(|c| c.link) {
                let tracks = groups.entry(link).or_default();
                if tracks.contains(&track.id) {
                    return Err(ValidationError::LinkSameTrack(link));
                }
                tracks.push(track.id);
            }
        }
        match groups.into_iter().find(|(_, tracks)| tracks.len() < 2) {
            Some((link, _)) => Err(ValidationError::LinkTooSmall(link)),
            None => Ok(()),
        }
    }
}

/// Positive durations, times within the IPC range, sorted and non-overlapping.
fn validate_clip_times(track: &Track) -> Result<(), ValidationError> {
    let mut previous: Option<&Clip> = None;
    for clip in &track.clips {
        if clip.duration.flicks() <= 0 {
            return Err(ValidationError::ClipNotPositive { clip: clip.id });
        }
        if clip.start.flicks() < 0 {
            return Err(ValidationError::ClipNegativeStart { clip: clip.id });
        }
        let in_range = clip
            .start
            .flicks()
            .checked_add(clip.duration.flicks())
            .is_some_and(|end| end <= MAX_IPC_FLICKS);
        if !in_range {
            return Err(ValidationError::TimeOutOfRange { clip: clip.id });
        }
        if let Some(previous) = previous {
            if clip.start < previous.start {
                return Err(ValidationError::ClipsOutOfOrder {
                    track: track.id,
                    clip: clip.id,
                });
            }
            if previous.end() > clip.start {
                return Err(ValidationError::ClipsOverlap {
                    track: track.id,
                    first: previous.id,
                    second: clip.id,
                });
            }
        }
        previous = Some(clip);
    }
    Ok(())
}

/// The clip/track compatibility table (§3.3), the media references and ranges, and the props.
fn validate_clip(
    kind: TrackKind,
    clip: &Clip,
    media: &HashMap<MediaId, &crate::media::MediaAsset>,
) -> Result<(), ValidationError> {
    let mismatch = ValidationError::ClipTrackMismatch {
        clip: clip.id,
        kind,
    };
    // `visual` is Some exactly on video and text tracks; audio-track clips always make sound.
    let draws = kind != TrackKind::Audio;
    if clip.visual.is_some() != draws || (kind == TrackKind::Audio && clip.audio.is_none()) {
        return Err(mismatch);
    }
    match (&clip.source, kind) {
        (ClipSource::Color { .. }, TrackKind::Video)
        | (ClipSource::Text { .. }, TrackKind::Text) => {
            if clip.audio.is_some() {
                return Err(mismatch);
            }
        }
        (
            &ClipSource::Media {
                media: id,
                source_in,
            },
            TrackKind::Video | TrackKind::Audio,
        ) => {
            let asset = media.get(&id).ok_or(ValidationError::ClipMediaMissing {
                clip: clip.id,
                media: id,
            })?;
            let has_audio = asset.kind != MediaKind::Image && !asset.info.audio.is_empty();
            let kind_ok = match kind {
                TrackKind::Video => asset.kind != MediaKind::Audio,
                _ => has_audio,
            };
            if !kind_ok || (clip.audio.is_some() && !has_audio) {
                return Err(ValidationError::ClipMediaMismatch {
                    clip: clip.id,
                    media: id,
                });
            }
            let range_ok = match asset.info.duration {
                None => source_in == Time::ZERO,
                Some(duration) => {
                    source_in >= Time::ZERO
                        && source_in
                            .flicks()
                            .checked_add(clip.duration.flicks())
                            .is_some_and(|end| end <= duration.flicks())
                }
            };
            if !range_ok {
                return Err(ValidationError::SourceRange { clip: clip.id });
            }
        }
        _ => return Err(mismatch),
    }
    let property = |property| ValidationError::PropertyRange {
        clip: clip.id,
        property,
    };
    if let Some(visual) = &clip.visual {
        visual.check(clip.duration).map_err(property)?;
    }
    if let Some(audio) = &clip.audio {
        audio.check(clip.duration).map_err(property)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FrameRate;

    fn clip(start: i64, duration: i64) -> Clip {
        Clip::color(
            "Clip",
            Time(start),
            Time(duration),
            crate::model::Rgba8::BLACK,
        )
    }

    /// A valid project with `clips` on its (non-magnetic) main track, the first track.
    fn with_clips(clips: Vec<Clip>) -> Project {
        let mut project = Project::new("Test");
        project.magnetic = false;
        project.tracks[0].clips = clips;
        project
    }

    fn settings(width: u32, height: u32, rate: (u32, u32), sample_rate: u32) -> ProjectSettings {
        ProjectSettings {
            width,
            height,
            frame_rate: FrameRate::new(rate.0, rate.1).expect("aligned rate"),
            sample_rate,
        }
    }

    #[test]
    fn a_new_project_is_valid() {
        assert_eq!(Project::new("Test").validate(), Ok(()));
        assert_eq!(
            with_clips(vec![clip(0, 10), clip(10, 5), clip(20, 1)]).validate(),
            Ok(())
        );
    }

    #[test]
    fn rejects_duplicate_ids_across_kinds() {
        let mut project = Project::new("Test");
        let id = project.tracks[0].id.0;
        project.tracks[1].id.0 = id;
        assert_eq!(project.validate(), Err(ValidationError::DuplicateId(id)));

        let mut project = with_clips(vec![clip(0, 10)]);
        project.tracks[0].clips[0].id.0 = project.id.0;
        assert_eq!(
            project.validate(),
            Err(ValidationError::DuplicateId(project.id.0))
        );
    }

    #[test]
    fn rejects_blank_names() {
        assert_eq!(
            Project::new(" \t").validate(),
            Err(ValidationError::EmptyName)
        );
    }

    #[test]
    fn rejects_clips_without_a_positive_duration() {
        for duration in [0, -1] {
            let project = with_clips(vec![clip(0, duration)]);
            let id = project.tracks[0].clips[0].id;
            assert_eq!(
                project.validate(),
                Err(ValidationError::ClipNotPositive { clip: id })
            );
        }
    }

    #[test]
    fn rejects_clips_before_zero() {
        let project = with_clips(vec![clip(-1, 10)]);
        let id = project.tracks[0].clips[0].id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipNegativeStart { clip: id })
        );
    }

    #[test]
    fn rejects_times_beyond_the_ipc_range() {
        assert_eq!(
            with_clips(vec![clip(MAX_IPC_FLICKS - 10, 10)]).validate(),
            Ok(())
        );
        for (start, duration) in [(MAX_IPC_FLICKS - 10, 11), (i64::MAX, 1), (1, i64::MAX)] {
            let project = with_clips(vec![clip(start, duration)]);
            let id = project.tracks[0].clips[0].id;
            assert_eq!(
                project.validate(),
                Err(ValidationError::TimeOutOfRange { clip: id }),
                "{start} + {duration}"
            );
        }
    }

    #[test]
    fn rejects_unsorted_clips() {
        let project = with_clips(vec![clip(20, 5), clip(0, 5)]);
        let track = &project.tracks[0];
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipsOutOfOrder {
                track: track.id,
                clip: track.clips[1].id
            })
        );
    }

    #[test]
    fn rejects_overlapping_clips() {
        let project = with_clips(vec![clip(0, 11), clip(10, 5)]);
        let track = &project.tracks[0];
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipsOverlap {
                track: track.id,
                first: track.clips[0].id,
                second: track.clips[1].id
            })
        );
    }

    #[test]
    fn clips_on_different_tracks_may_overlap() {
        let mut project = with_clips(vec![clip(0, 10)]);
        let mut overlay = Track::new(TrackKind::Video, "Video 2");
        overlay.clips = vec![clip(5, 10)];
        project.tracks.insert(0, overlay);
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn clip_times_need_not_lie_on_the_frame_grid() {
        assert_eq!(with_clips(vec![clip(1, 7)]).validate(), Ok(()));
    }

    #[test]
    fn canvas_size_edges() {
        for (w, h) in [(16, 16), (8192, 8192), (1080, 1920)] {
            assert_eq!(settings(w, h, (30, 1), 48_000).validate(), Ok(()));
        }
        for (w, h) in [
            (14, 16),
            (16, 14),
            (8194, 1080),
            (1920, 8194),
            (1921, 1080),
            (0, 0),
        ] {
            assert_eq!(
                settings(w, h, (30, 1), 48_000).validate(),
                Err(ValidationError::CanvasSize {
                    width: w,
                    height: h
                }),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn frame_rate_edges() {
        for rate in [
            (1, 1),
            (240, 1),
            (24000, 1001),
            (30000, 1001),
            (60000, 1001),
        ] {
            assert_eq!(settings(1920, 1080, rate, 48_000).validate(), Ok(()));
        }
        // 1/2 fps and 250 fps are flick-aligned but out of range.
        for (num, den) in [(1, 2), (250, 1), (480, 1)] {
            assert_eq!(
                settings(1920, 1080, (num, den), 48_000).validate(),
                Err(ValidationError::FrameRateRange { num, den })
            );
        }
    }

    #[test]
    fn sample_rate_edges() {
        for rate in SAMPLE_RATES {
            assert_eq!(settings(1920, 1080, (30, 1), rate).validate(), Ok(()));
        }
        assert_eq!(
            settings(1920, 1080, (30, 1), 22_050).validate(),
            Err(ValidationError::SampleRate(22_050))
        );
    }

    #[test]
    fn project_validation_includes_settings() {
        let mut project = Project::new("Test");
        project.settings.sample_rate = 96_000;
        assert_eq!(project.validate(), Err(ValidationError::SampleRate(96_000)));
    }

    #[test]
    fn project_validation_includes_media() {
        use crate::media::{MediaProblem, samples};

        let mut project = Project::new("Test");
        project.media.push(samples::video(1));
        assert_eq!(project.validate(), Ok(()));
        let mut broken = samples::audio(2);
        broken.path.absolute.clear();
        let id = broken.id;
        project.media.push(broken);
        assert_eq!(
            project.validate(),
            Err(ValidationError::Media(MediaValidationError {
                id,
                problem: MediaProblem::EmptyPath,
            }))
        );
    }

    /// Overlay text and video tracks, the main track with a media clip linked to its detached
    /// audio, and an audio track: everything valid.
    fn timeline() -> Project {
        use crate::media::samples;
        use crate::model::{ClipSource, LinkId};
        use crate::props::AudioProps;

        let mut project = Project::new("Timeline");
        let (video, song, photo) = (samples::video(1), samples::audio(2), samples::image(3));
        let link = LinkId::new();
        let media = |name: &str, media: MediaId, source_in: i64, duration: i64| Clip {
            source: ClipSource::Media {
                media,
                source_in: Time(source_in),
            },
            ..clip_named(name, 0, duration)
        };
        let mut main_clip = media("Main", video.id, 10, 1000);
        main_clip.link = Some(link);
        let mut detached = media("Detached", video.id, 10, 1000);
        detached.link = Some(link);
        detached.visual = None;
        detached.audio = Some(AudioProps::default());
        let mut music = media("Song", song.id, 0, 2000);
        music.visual = None;
        music.audio = Some(AudioProps::default());
        let mut text = Track::new(TrackKind::Text, "Text 1");
        text.clips = vec![Clip::text("Title", Time(0), Time(500), "Hello")];
        let mut overlay = Track::new(TrackKind::Video, "Video 2");
        overlay.clips = vec![media("Photo", photo.id, 0, 7000)];
        project.tracks[0].clips = vec![main_clip, clip(1000, 50)];
        project.tracks[1].clips = vec![detached];
        let mut music_track = Track::new(TrackKind::Audio, "Audio 2");
        music_track.clips = vec![music];
        project.tracks.insert(0, overlay);
        project.tracks.insert(0, text);
        project.tracks.push(music_track);
        project.media = vec![video, song, photo];
        project.background = CanvasBackground::Image {
            media: project.media[2].id,
        };
        project
    }

    fn clip_named(name: &str, start: i64, duration: i64) -> Clip {
        Clip::color(
            name,
            Time(start),
            Time(duration),
            crate::model::Rgba8::BLACK,
        )
    }

    /// The clip named `name`, to break in a test.
    fn named<'a>(project: &'a mut Project, name: &str) -> &'a mut Clip {
        project
            .tracks
            .iter_mut()
            .flat_map(|t| t.clips.iter_mut())
            .find(|c| c.name == name)
            .expect("clip exists")
    }

    #[test]
    fn the_timeline_sample_is_valid() {
        assert_eq!(timeline().validate(), Ok(()));
    }

    #[test]
    fn main_track_must_be_an_existing_video_track() {
        let mut project = timeline();
        project.main_track = TrackId::new();
        assert_eq!(
            project.validate(),
            Err(ValidationError::MainTrack(project.main_track))
        );
        let mut project = timeline();
        project.main_track = project.tracks[0].id; // the text track
        assert_eq!(
            project.validate(),
            Err(ValidationError::MainTrack(project.main_track))
        );
    }

    #[test]
    fn tracks_must_be_in_section_order() {
        let mut project = timeline();
        let audio = project.tracks.pop().unwrap();
        let id = audio.id;
        project.tracks.insert(0, audio);
        assert_eq!(project.validate(), Err(ValidationError::TrackSection(id)));

        let mut project = timeline();
        let text = project.tracks.remove(0);
        let id = text.id;
        project.tracks.push(text);
        assert_eq!(project.validate(), Err(ValidationError::TrackSection(id)));
    }

    #[test]
    fn clip_sources_and_props_must_fit_the_track() {
        type Edit = fn(&mut Clip);
        let cases: [(&str, Edit); 5] = [
            ("Title", |c| c.visual = None),
            ("Photo", |c| {
                c.audio = Some(crate::props::AudioProps::default())
            }),
            ("Song", |c| c.audio = None),
            ("Detached", |c| {
                c.visual = Some(crate::props::VisualProps::default())
            }),
            ("Title", |c| {
                c.source = crate::model::ClipSource::Color {
                    color: crate::model::Rgba8::BLACK,
                }
            }),
        ];
        for (name, edit) in cases {
            let mut project = timeline();
            let clip = named(&mut project, name);
            edit(clip);
            let id = clip.id;
            let err = project.validate().unwrap_err();
            assert!(
                matches!(&err, ValidationError::ClipTrackMismatch { clip, .. } if *clip == id)
                    || matches!(&err, ValidationError::ClipMediaMismatch { clip, .. } if *clip == id),
                "{name}: {err:?}"
            );
        }
        // A colour card on an audio track.
        let mut project = timeline();
        let mut card = clip(5000, 10);
        card.visual = None;
        card.audio = Some(crate::props::AudioProps::default());
        let id = card.id;
        project.tracks.last_mut().unwrap().clips.push(card);
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipTrackMismatch {
                clip: id,
                kind: TrackKind::Audio
            })
        );
    }

    #[test]
    fn media_clips_must_match_their_media() {
        use crate::model::ClipSource;

        let mut project = timeline();
        let missing = MediaId::new();
        let clip = named(&mut project, "Photo");
        clip.source = ClipSource::Media {
            media: missing,
            source_in: Time::ZERO,
        };
        let id = clip.id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipMediaMissing {
                clip: id,
                media: missing
            })
        );

        // Audio-only media on a video track.
        let mut project = timeline();
        let song = project.media[1].id;
        let clip = named(&mut project, "Photo");
        clip.source = ClipSource::Media {
            media: song,
            source_in: Time::ZERO,
        };
        let id = clip.id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::ClipMediaMismatch {
                clip: id,
                media: song
            })
        );

        // A video without audio cannot have audio props.
        let mut project = timeline();
        project.media[0].info.audio.clear();
        let video = project.media[0].id;
        let err = project.validate().unwrap_err();
        assert!(
            matches!(err, ValidationError::ClipMediaMismatch { media, .. } if media == video),
            "{err:?}"
        );
    }

    #[test]
    fn source_ranges_must_lie_inside_the_media() {
        use crate::model::ClipSource;

        let five_seconds = Time::from_seconds(5).flicks();
        let cases: [(&str, i64, i64, bool); 6] = [
            ("Main", 0, five_seconds, true),
            ("Main", 1, five_seconds, false),
            ("Main", -1, 10, false),
            ("Photo", 0, 1 << 40, true),
            ("Photo", 1, 10, false),
            ("Song", 0, Time::from_seconds(3).flicks() + 1, false),
        ];
        for (name, source_in, duration, ok) in cases {
            let mut project = timeline();
            project.magnetic = false;
            for clip in project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
                clip.link = None;
            }
            let clip = named(&mut project, name);
            if let ClipSource::Media { source_in: s, .. } = &mut clip.source {
                *s = Time(source_in);
            }
            clip.start = Time(1 << 41);
            clip.duration = Time(duration);
            let id = clip.id;
            for track in &mut project.tracks {
                track.clips.sort_by_key(|c| c.start);
            }
            let expected = if ok {
                Ok(())
            } else {
                Err(ValidationError::SourceRange { clip: id })
            };
            assert_eq!(
                project.validate(),
                expected,
                "{name} {source_in} {duration}"
            );
        }
    }

    #[test]
    fn magnetic_main_track_is_contiguous_from_zero() {
        let mut project = timeline();
        let main = project.main_index().unwrap();
        project.tracks[main].clips[1].start = Time(1001);
        let id = project.tracks[main].clips[1].id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::MainTrackNotContiguous { clip: id })
        );
        project.magnetic = false;
        assert_eq!(project.validate(), Ok(()));

        let mut project = with_clips(vec![clip(1, 10)]);
        project.magnetic = true;
        let id = project.tracks[0].clips[0].id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::MainTrackNotContiguous { clip: id })
        );
    }

    #[test]
    fn links_need_two_clips_on_different_tracks() {
        let mut project = timeline();
        let link = named(&mut project, "Main").link.unwrap();
        named(&mut project, "Detached").link = None;
        assert_eq!(project.validate(), Err(ValidationError::LinkTooSmall(link)));

        let mut project = timeline();
        let link = named(&mut project, "Main").link.unwrap();
        let main = project.main_index().unwrap();
        project.tracks[main].clips[1].link = Some(link);
        assert_eq!(
            project.validate(),
            Err(ValidationError::LinkSameTrack(link))
        );
    }

    #[test]
    fn property_ranges_are_checked() {
        let mut project = timeline();
        let clip = named(&mut project, "Photo");
        clip.visual.as_mut().unwrap().opacity.value = 2.0;
        let id = clip.id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::PropertyRange {
                clip: id,
                property: "opacity"
            })
        );
        let mut project = timeline();
        let clip = named(&mut project, "Song");
        clip.audio.as_mut().unwrap().fade_in = Time(1500);
        clip.audio.as_mut().unwrap().fade_out = Time(501);
        let id = clip.id;
        assert_eq!(
            project.validate(),
            Err(ValidationError::PropertyRange {
                clip: id,
                property: "fades"
            })
        );
    }

    #[test]
    fn background_is_checked() {
        let mut project = timeline();
        project.background = CanvasBackground::Image {
            media: project.media[0].id,
        };
        assert_eq!(project.validate(), Err(ValidationError::Background));
        project.background = CanvasBackground::Blur { amount: 1.5 };
        assert_eq!(project.validate(), Err(ValidationError::Background));
        project.background = CanvasBackground::Blur { amount: f32::NAN };
        assert_eq!(project.validate(), Err(ValidationError::Background));
        project.background = CanvasBackground::Blur { amount: 0.4 };
        assert_eq!(project.validate(), Ok(()));
    }
}
