//! The project model. Plain data: mutation happens only through [`crate::command::Command`]s.

use serde::{Deserialize, Serialize};

use crate::media::{MediaAsset, MediaId};
use crate::props::{AudioProps, VisualProps};
use crate::time::{FrameRate, Time};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize)]
        #[cfg_attr(feature = "specta", derive(specta::Type))]
        #[serde(transparent)]
        pub struct $name(pub ::uuid::Uuid);

        impl $name {
            /// A new time-ordered (UUID v7) identifier.
            pub fn new() -> Self {
                Self(::uuid::Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}
pub(crate) use id_type;

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
id_type!(
    /// Shared by the clips of one link group: clips that move, trim, split and delete together
    /// (design `timeline-editing.md` §3.5). Not a unique id: every member carries the same value.
    LinkId
);

/// Default length of a new image clip (F1.83 later takes it from preferences).
pub const DEFAULT_IMAGE_DURATION: Time = Time::from_seconds(5);
/// Default length of a new colour clip.
pub const DEFAULT_COLOR_DURATION: Time = Time::from_seconds(5);
/// Commands that create or shorten clips keep at least this many project frames.
pub const MIN_CLIP_FRAMES: i64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub schema_version: u32,
    pub id: ProjectId,
    pub name: String,
    pub settings: ProjectSettings,
    /// What fills the canvas behind the main track (F1.76).
    pub background: CanvasBackground,
    /// The magnetic main track: always an existing video track (§3.3).
    pub main_track: TrackId,
    /// While on, the main track has no gaps: its clips are contiguous from time 0.
    pub magnetic: bool,
    /// Top to bottom as shown in the timeline: overlay tracks (video, text), then the main track,
    /// then audio tracks. Higher tracks composite over lower ones.
    pub tracks: Vec<Track>,
    /// Imported media, in import order. The media panel sorts and filters its own view of it.
    pub media: Vec<MediaAsset>,
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
    /// Edits never change this track's clips, and its clips never follow the main track.
    pub locked: bool,
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
            locked: false,
            clips: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: ClipId,
    pub name: String,
    /// Timeline position and length. `end() = start + duration`.
    pub start: Time,
    pub duration: Time,
    pub source: ClipSource,
    pub link: Option<LinkId>,
    /// Present exactly on clips of video and text tracks.
    pub visual: Option<VisualProps>,
    /// Present on clips that make sound: audio-track clips, and video-track media clips whose
    /// media has audio that was not detached.
    pub audio: Option<AudioProps>,
}

/// What a clip shows or plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClipSource {
    /// A video, audio or image file. `source_in` is the media time at the clip's start, in
    /// flicks (seeked by timestamp, never a frame index); always 0 for images.
    #[serde(rename_all = "camelCase")]
    Media { media: MediaId, source_in: Time },
    /// A solid colour card.
    Color { color: Rgba8 },
    /// A title. Task 0510 adds the style fields and the rendering.
    Text { content: String },
}

impl ClipSource {
    pub fn media(&self) -> Option<MediaId> {
        match self {
            ClipSource::Media { media, .. } => Some(*media),
            _ => None,
        }
    }
}

impl Clip {
    /// A colour card for a video track, with default visual props.
    pub fn color(name: impl Into<String>, start: Time, duration: Time, color: Rgba8) -> Self {
        Self {
            id: ClipId::new(),
            name: name.into(),
            start,
            duration,
            source: ClipSource::Color { color },
            link: None,
            visual: Some(VisualProps::default()),
            audio: None,
        }
    }

    /// A title for a text track, with default visual props.
    pub fn text(
        name: impl Into<String>,
        start: Time,
        duration: Time,
        content: impl Into<String>,
    ) -> Self {
        Self {
            source: ClipSource::Text {
                content: content.into(),
            },
            ..Self::color(name, start, duration, Rgba8::BLACK)
        }
    }

    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    /// `source_in` of a media clip; 0 for colour and text clips.
    pub fn source_in(&self) -> Time {
        match self.source {
            ClipSource::Media { source_in, .. } => source_in,
            _ => Time::ZERO,
        }
    }

    /// The media time shown or played at timeline time `t`: `source_in + (t - start)` for `t` in
    /// `start..end()`. `None` outside the clip and for colour and text clips. The only place the
    /// timeline-to-source mapping lives; task 0370 adds speed here.
    pub fn source_time(&self, t: Time) -> Option<Time> {
        match self.source {
            ClipSource::Media { source_in, .. } if self.start <= t && t < self.end() => {
                Some(source_in + (t - self.start))
            }
            _ => None,
        }
    }

    /// End of the used source range (exclusive) of a media clip: `source_in + duration`.
    pub fn source_end(&self) -> Option<Time> {
        match self.source {
            ClipSource::Media { source_in, .. } => Some(source_in + self.duration),
            _ => None,
        }
    }
}

/// An sRGB colour with straight alpha, written as `"#rrggbbaa"` (lowercase hex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(try_from = "String", into = "String")]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const BLACK: Rgba8 = Rgba8::rgb(0, 0, 0);
    /// The colour v1/v2 clips on video tracks become in the v3 migration.
    pub const MIGRATED_GRAY: Rgba8 = Rgba8::rgb(0x80, 0x80, 0x80);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

impl std::fmt::Display for Rgba8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}{:02x}",
            self.r, self.g, self.b, self.a
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a #rrggbbaa colour")]
pub struct ColorParseError(String);

impl std::str::FromStr for Rgba8 {
    type Err = ColorParseError;

    /// Exactly `#` and 8 hex digits; upper case is accepted, written colours are lower case.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || ColorParseError(s.to_owned());
        let hex = s
            .strip_prefix('#')
            .filter(|h| h.len() == 8 && h.is_ascii())
            .ok_or_else(error)?;
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| error());
        Ok(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: byte(6)?,
        })
    }
}

impl TryFrom<String> for Rgba8 {
    type Error = ColorParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Rgba8> for String {
    fn from(color: Rgba8) -> Self {
        color.to_string()
    }
}

/// What fills the canvas behind the main track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CanvasBackground {
    Color {
        color: Rgba8,
    },
    /// The frame of the main-track clip itself, blurred, behind it. `amount` is 0..=1.
    Blur {
        amount: f32,
    },
    /// An image media asset.
    Image {
        media: MediaId,
    },
}

impl Default for CanvasBackground {
    /// Opaque black.
    fn default() -> Self {
        CanvasBackground::Color {
            color: Rgba8::BLACK,
        }
    }
}

impl Project {
    /// A new project: an empty magnetic main track ("Video 1") and one audio track.
    pub fn new(name: impl Into<String>) -> Self {
        let main = Track::new(TrackKind::Video, "Video 1");
        Self {
            schema_version: crate::serialization::CURRENT_SCHEMA_VERSION,
            id: ProjectId::new(),
            name: name.into(),
            settings: ProjectSettings::default(),
            background: CanvasBackground::default(),
            main_track: main.id,
            magnetic: true,
            tracks: vec![main, Track::new(TrackKind::Audio, "Audio 1")],
            media: Vec::new(),
        }
    }

    /// Index of the main track in `tracks`. `None` only in a project that breaks the invariants.
    pub fn main_index(&self) -> Option<usize> {
        self.track_index(self.main_track)
    }

    /// The main track. `None` only in a project that breaks the invariants.
    pub fn main(&self) -> Option<&Track> {
        self.track(self.main_track)
    }

    /// The clip with `id` and the index of its track.
    pub fn clip(&self, id: ClipId) -> Option<(usize, &Clip)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(index, track)| track.clips.iter().find(|c| c.id == id).map(|c| (index, c)))
    }

    /// How many clips reference media `id`.
    pub fn media_users(&self, id: MediaId) -> usize {
        self.clips()
            .filter(|c| c.source.media() == Some(id))
            .count()
    }

    /// Every clip of every track, top track first.
    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|t| t.clips.iter())
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    pub fn media_asset(&self, id: crate::media::MediaId) -> Option<&MediaAsset> {
        self.media.iter().find(|m| m.id == id)
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
