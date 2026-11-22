//! Invariants of a [`Project`], checked when a file is loaded (ADR-0002 §3) and when settings change.
//!
//! An invalid project is reported, never repaired: every command may assume these invariants hold.

use std::collections::HashSet;

use uuid::Uuid;

use crate::media::{MediaValidationError, validate_media};
use crate::model::{ClipId, Project, ProjectSettings, TrackId};
use crate::time::MAX_IPC_FLICKS;

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
    /// Checks every invariant: settings ranges, a non-blank name, unique ids, and clips that have
    /// a positive duration, lie within `0..=MAX_IPC_FLICKS` and are sorted and non-overlapping on
    /// each track. Clip times need not lie on the frame grid, so a frame-rate change never makes a
    /// project invalid. Media assets are checked by [`validate_media`]; their ids are unique among
    /// media (uuid v7 makes a clash with a track or clip id practically impossible).
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName);
        }
        self.settings.validate()?;

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
            let mut previous: Option<&crate::model::Clip> = None;
            for clip in &track.clips {
                unique(clip.id.0)?;
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
        }
        validate_media(&self.media)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Clip;
    use crate::time::{FrameRate, Time};

    fn clip(start: i64, duration: i64) -> Clip {
        Clip {
            id: ClipId::new(),
            name: "Clip".into(),
            start: Time(start),
            duration: Time(duration),
        }
    }

    /// A valid project with `clips` on its first track.
    fn with_clips(clips: Vec<Clip>) -> Project {
        let mut project = Project::new("Test");
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
        project.tracks[1].clips = vec![clip(5, 10)];
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
}
