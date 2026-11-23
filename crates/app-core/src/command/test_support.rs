//! Fixtures shared by the tests of the cut, clipboard and sequence commands.

use super::{Command, CommandError};
use crate::Document;
use crate::media::samples;
use crate::model::{Clip, ClipId, ClipSource, LinkId, Project, Rgba8, Track, TrackKind};
use crate::props::{AudioProps, VisualProps};
use crate::time::{FLICKS_PER_SECOND, Time};
use crate::timeline::place_clip;

pub const S: i64 = FLICKS_PER_SECOND;

pub fn t(seconds: f64) -> Time {
    Time((seconds * S as f64).round() as i64)
}

/// Apply → revert gives the original, re-apply gives the same result. Returns that result.
pub fn assert_reversible(command: &mut dyn Command, project: &Project) -> Project {
    let mut edited = project.clone();
    command.apply(&mut edited).expect("apply");
    assert_eq!(edited.validate(), Ok(()));
    let after = edited.clone();
    command.revert(&mut edited);
    assert_eq!(&edited, project, "revert restores the original");
    command.apply(&mut edited).expect("re-apply");
    assert_eq!(edited, after, "re-apply reproduces the edit");
    after
}

/// Runs `command` through a document and checks that a failure changes nothing.
pub fn try_apply(
    project: &Project,
    command: impl Command + 'static,
) -> Result<Project, CommandError> {
    let mut doc = Document::new(project.clone());
    let result = doc.execute(Box::new(command));
    if result.is_err() {
        assert_eq!(doc.project(), project, "a failed edit changes nothing");
    }
    result.map(|()| doc.project().clone())
}

pub const OVERLAY: usize = 0;
pub const MAIN: usize = 1;
pub const AUDIO: usize = 2;

/// Tracks: 0 overlay "Video 2", 1 main "Video 1", 2 "Audio 1". Media: 5 s video with audio, 3 s
/// song, a photo. No clips. Magnetic.
pub fn empty() -> Project {
    let mut project = Project::new("Cut");
    project.media = vec![samples::video(1), samples::audio(2), samples::image(3)];
    project
        .tracks
        .insert(0, Track::new(TrackKind::Video, "Video 2"));
    project
}

pub fn color(start: f64, duration: f64) -> Clip {
    Clip::color("card", t(start), t(duration), Rgba8::MIGRATED_GRAY)
}

/// A clip of the 5 s video (with its sound) on a video track.
pub fn video(project: &Project, start: f64, duration: f64, source_in: f64) -> Clip {
    Clip {
        id: ClipId::new(),
        name: "clip".into(),
        start: t(start),
        duration: t(duration),
        source: ClipSource::Media {
            media: project.media[0].id,
            source_in: t(source_in),
        },
        link: None,
        visual: Some(VisualProps::default()),
        audio: Some(AudioProps::default()),
    }
}

/// The video's sound on an audio track (as a detached clip would be).
pub fn sound(project: &Project, start: f64, duration: f64, source_in: f64) -> Clip {
    Clip {
        visual: None,
        ..video(project, start, duration, source_in)
    }
}

pub fn image(project: &Project, start: f64, duration: f64) -> Clip {
    Clip {
        source: ClipSource::Media {
            media: project.media[2].id,
            source_in: Time::ZERO,
        },
        audio: None,
        ..video(project, start, duration, 0.0)
    }
}

pub trait PushClip {
    fn push_clip(&mut self, track: usize, clip: Clip) -> ClipId;
}

impl PushClip for Project {
    fn push_clip(&mut self, track: usize, clip: Clip) -> ClipId {
        let id = clip.id;
        place_clip(&mut self.tracks[track], clip).expect("free");
        id
    }
}

pub fn link(project: &mut Project, ids: &[ClipId]) {
    let link = LinkId::new();
    for clip in project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if ids.contains(&clip.id) {
            clip.link = Some(link);
        }
    }
}

/// Main track: A, B, C, 5 s colour cards each.
pub fn three_on_main() -> (Project, [ClipId; 3]) {
    let mut p = empty();
    let a = p.push_clip(MAIN, color(0.0, 5.0));
    let b = p.push_clip(MAIN, color(5.0, 5.0));
    let c = p.push_clip(MAIN, color(10.0, 5.0));
    assert_eq!(p.validate(), Ok(()));
    (p, [a, b, c])
}

pub fn ids(track: &Track) -> Vec<ClipId> {
    track.clips.iter().map(|c| c.id).collect()
}

pub fn starts(track: &Track) -> Vec<Time> {
    track.clips.iter().map(|c| c.start).collect()
}

pub fn clip(project: &Project, id: ClipId) -> &Clip {
    project.clip(id).expect("clip exists").1
}

pub fn track_of(project: &Project, id: ClipId) -> usize {
    project.clip(id).expect("clip exists").0
}
