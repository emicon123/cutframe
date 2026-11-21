//! The project model. Plain data: mutation happens only through [`crate::command::Command`]s.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::{FrameRate, Time};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "specta", derive(specta::Type))]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// A new time-ordered (UUID v7) identifier.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

id_type!(
    /// Identifies a project across saves.
    ProjectId
);
id_type!(
    /// Identifies a track within a project.
    TrackId
);
id_type!(
    /// Identifies a clip within a project.
    ClipId
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub schema_version: u32,
    pub id: ProjectId,
    pub name: String,
    pub settings: ProjectSettings,
    /// Top to bottom as shown in the timeline. Higher tracks composite over lower ones.
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub sample_rate: u32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::FPS_30,
            sample_rate: 48_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub muted: bool,
    pub hidden: bool,
    /// Sorted by `start`, never overlapping.
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(kind: TrackKind, name: impl Into<String>) -> Self {
        Self {
            id: TrackId::new(),
            kind,
            name: name.into(),
            muted: false,
            hidden: false,
            clips: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: ClipId,
    pub name: String,
    /// Position on the timeline.
    pub start: Time,
    pub duration: Time,
}

impl Clip {
    pub fn end(&self) -> Time {
        self.start + self.duration
    }
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema_version: crate::serialization::CURRENT_SCHEMA_VERSION,
            id: ProjectId::new(),
            name: name.into(),
            settings: ProjectSettings::default(),
            tracks: vec![
                Track::new(TrackKind::Video, "Video 1"),
                Track::new(TrackKind::Audio, "Audio 1"),
            ],
        }
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// End of the last clip on any track.
    pub fn duration(&self) -> Time {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(Clip::end))
            .max()
            .unwrap_or(Time::ZERO)
    }

    /// The next free default name for a track of `kind`, e.g. "Video 2".
    pub fn next_track_name(&self, kind: TrackKind) -> String {
        let prefix = match kind {
            TrackKind::Video => "Video",
            TrackKind::Audio => "Audio",
            TrackKind::Text => "Text",
        };
        let count = self.tracks.iter().filter(|t| t.kind == kind).count();
        (count + 1..)
            .map(|n| format!("{prefix} {n}"))
            .find(|name| self.tracks.iter().all(|t| &t.name != name))
            .unwrap_or_else(|| prefix.to_owned())
    }
}
