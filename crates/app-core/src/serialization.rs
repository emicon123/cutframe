//! Project file (de)serialization with schema versioning.
//!
//! Every format change bumps [`CURRENT_SCHEMA_VERSION`] and adds a step to [`migrate`], with a test
//! that loads a fixture written by the previous version. Reading and writing files is the shell's
//! job: this module converts to and from strings only (ADR-0002 §8).

use serde_json::Value;

use crate::model::Project;
use crate::validate::ValidationError;

pub const CURRENT_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("project file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `schemaVersion` is missing, not a number, or 0 (versions start at 1).
    #[error("project file has no valid schemaVersion")]
    MissingVersion,
    #[error("project file version {found} is newer than this app supports ({supported})")]
    TooNew { found: u64, supported: u32 },
    /// The file parsed but breaks a model invariant (ADR-0002 §3). It is never repaired silently.
    #[error("invalid project: {0}")]
    Invalid(#[from] ValidationError),
    /// An older file holds something the current format cannot represent (e.g. a v1/v2 clip on
    /// an audio track, which has no media to play).
    #[error("cannot migrate project from version {from}: {reason}")]
    Migration { from: u32, reason: &'static str },
}

/// A project read from a file, with the schema version the file was written in.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// Migrated to [`CURRENT_SCHEMA_VERSION`] and validated.
    pub project: Project,
    /// `schemaVersion` of the file. Lower than current means the first save makes a backup.
    pub source_version: u32,
}

/// Serializes `project` in the current schema version.
pub fn to_json(project: &Project) -> String {
    debug_assert_eq!(
        project.schema_version, CURRENT_SCHEMA_VERSION,
        "only current-version projects are saved"
    );
    serde_json::to_string_pretty(project).expect("project model is always serializable")
}

/// The load pipeline (ADR-0002 §3): parse → read `schemaVersion` → reject newer versions →
/// migrate step by step → deserialize → validate.
pub fn load(text: &str) -> Result<Loaded, LoadError> {
    let value: Value = serde_json::from_str(text)?;
    let source_version = schema_version(&value)?;
    let value = migrate(value, source_version)?;
    let project: Project = serde_json::from_value(value)?;
    debug_assert_eq!(
        project.schema_version, CURRENT_SCHEMA_VERSION,
        "every migration step sets schemaVersion to the version it produces"
    );
    project.validate()?;
    Ok(Loaded {
        project,
        source_version,
    })
}

/// [`load`] without the source version.
pub fn from_json(text: &str) -> Result<Project, LoadError> {
    load(text).map(|loaded| loaded.project)
}

/// The file's `schemaVersion`, if this app can read it.
fn schema_version(value: &Value) -> Result<u32, LoadError> {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .filter(|&v| v >= 1)
        .ok_or(LoadError::MissingVersion)?;
    if version > u64::from(CURRENT_SCHEMA_VERSION) {
        return Err(LoadError::TooNew {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    // Lossless: at most CURRENT_SCHEMA_VERSION.
    Ok(version as u32)
}

/// Upgrades a project document of a supported older `version` to the current one. Each step sets
/// `schemaVersion` to the version it produces.
fn migrate(mut value: Value, version: u32) -> Result<Value, LoadError> {
    if version < 2 {
        value = v1_to_v2(value);
    }
    if version < 3 {
        value = v2_to_v3(value).map_err(|reason| LoadError::Migration {
            from: version,
            reason,
        })?;
    }
    if version < 4 {
        value = v3_to_v4(value);
    }
    Ok(value)
}

/// v4 adds speed, reverse and pitch (design `clip-speed-audio.md` §7): every media source gains
/// `speed: 100, reverse: false` and every audio prop `preservePitch: true`. Nothing else changes,
/// so no clip can become invalid.
fn v3_to_v4(mut value: Value) -> Value {
    let Some(project) = value.as_object_mut() else {
        return value;
    };
    let clips = project
        .get_mut("tracks")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get_mut("clips").and_then(Value::as_array_mut))
        .flatten();
    for clip in clips {
        if let Some(source) = clip.get_mut("source").and_then(Value::as_object_mut)
            && source.get("kind").and_then(Value::as_str) == Some("media")
        {
            source.insert("speed".to_owned(), Value::from(100));
            source.insert("reverse".to_owned(), Value::Bool(false));
        }
        if let Some(audio) = clip.get_mut("audio").and_then(Value::as_object_mut) {
            audio.insert("preservePitch".to_owned(), Value::Bool(true));
        }
    }
    project.insert("schemaVersion".to_owned(), Value::from(4));
    value
}

/// v2 adds the media list (design `media-import.md` §8). A v1 project has no media.
fn v1_to_v2(mut value: Value) -> Value {
    if let Some(project) = value.as_object_mut() {
        project.insert("media".to_owned(), Value::Array(Vec::new()));
        project.insert("schemaVersion".to_owned(), Value::from(2));
    }
    value
}

/// v3 adds the clip and track model of design `timeline-editing.md` §3 (§9): track locks, the main
/// track and the track sections, clip sources and props, links, the canvas background and the
/// magnetic flag. Shapes this step does not recognise are left for deserialization to reject.
fn v2_to_v3(mut value: Value) -> Result<Value, &'static str> {
    use serde_json::json;

    use crate::model::{ClipSource, Rgba8};
    use crate::props::VisualProps;

    let Some(project) = value.as_object_mut() else {
        return Ok(value);
    };
    let Some(Value::Array(tracks)) = project.get_mut("tracks") else {
        return Ok(value);
    };
    let kind = |track: &Value| track.get("kind").and_then(Value::as_str).map(str::to_owned);
    let visual = serde_json::to_value(VisualProps::default()).expect("props serialize");
    for track in tracks.iter_mut() {
        let track_kind = kind(track);
        let Some(track) = track.as_object_mut() else {
            continue;
        };
        track.insert("locked".to_owned(), Value::Bool(false));
        let Some(Value::Array(clips)) = track.get_mut("clips") else {
            continue;
        };
        for clip in clips.iter_mut().filter_map(Value::as_object_mut) {
            // No released build could create clips before v3: they exist only in hand-written
            // files, without media. Video clips become grey cards, text clips titles.
            let source = match track_kind.as_deref() {
                Some("video") => ClipSource::Color {
                    color: Rgba8::MIGRATED_GRAY,
                },
                Some("text") => ClipSource::Text {
                    content: clip
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
                Some("audio") => return Err("a clip on an audio track has no media"),
                _ => continue,
            };
            let source = serde_json::to_value(source).expect("source serializes");
            clip.insert("source".to_owned(), source);
            clip.insert("link".to_owned(), Value::Null);
            clip.insert("visual".to_owned(), visual.clone());
            clip.insert("audio".to_owned(), Value::Null);
        }
    }

    // The main track is the last video track, or a new empty one.
    let main = match tracks
        .iter()
        .rposition(|t| kind(t).as_deref() == Some("video"))
    {
        Some(index) => index,
        None => {
            let names: Vec<&str> = tracks
                .iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str))
                .collect();
            let name = (1..)
                .map(|n| format!("Video {n}"))
                .find(|n| !names.contains(&n.as_str()))
                .expect("a free name exists");
            tracks.push(json!({
                "id": crate::model::TrackId::new(),
                "kind": "video",
                "name": name,
                "muted": false,
                "hidden": false,
                "locked": false,
                "clips": [],
            }));
            tracks.len() - 1
        }
    };
    let main_track = tracks[main].clone();
    let contiguous = {
        let mut end = Some(0i64);
        for clip in main_track["clips"].as_array().into_iter().flatten() {
            let start = clip.get("start").and_then(Value::as_i64);
            let duration = clip.get("duration").and_then(Value::as_i64);
            end = match (end, start, duration) {
                (Some(end), Some(start), Some(duration)) if start == end => {
                    end.checked_add(duration)
                }
                _ => None,
            };
        }
        end.is_some()
    };
    // Overlay (video, text) tracks keep their order above the main track, audio tracks below it.
    let mut overlays = Vec::new();
    let mut audio = Vec::new();
    for (index, track) in std::mem::take(tracks).into_iter().enumerate() {
        if index == main {
            continue;
        }
        if kind(&track).as_deref() == Some("audio") {
            audio.push(track);
        } else {
            overlays.push(track);
        }
    }
    let main_id = main_track.get("id").cloned().unwrap_or(Value::Null);
    *tracks = overlays
        .into_iter()
        .chain(std::iter::once(main_track))
        .chain(audio)
        .collect();

    project.insert("mainTrack".to_owned(), main_id);
    project.insert("magnetic".to_owned(), Value::Bool(contiguous));
    project.insert(
        "background".to_owned(),
        json!({ "kind": "color", "color": "#000000ff" }),
    );
    project.insert("schemaVersion".to_owned(), Value::from(3));
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_project() {
        let project = Project::new("Round trip");
        let loaded = from_json(&to_json(&project)).unwrap();
        assert_eq!(loaded, project);
    }

    #[test]
    fn loads_v1_fixture() {
        let project = from_json(include_str!("../tests/fixtures/project-v1.json")).unwrap();
        assert_eq!(project.name, "Fixture v1");
        assert_eq!(project.tracks.len(), 2);
        assert_eq!(
            project.tracks[0].clips[0].duration.flicks(),
            3 * crate::time::FLICKS_PER_SECOND
        );
    }

    #[test]
    fn v1_fixture_migrates_to_an_empty_media_list() {
        let project = from_json(include_str!("../tests/fixtures/project-v1.json")).unwrap();
        assert_eq!(project.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(project.media.is_empty());
    }

    #[test]
    fn loads_v2_fixture_and_round_trips_it() {
        use crate::media::{HdrTransfer, MediaKind, Rotation, validate_media};

        let project = from_json(include_str!("../tests/fixtures/project-v2.json")).unwrap();
        assert_eq!(project.name, "Fixture v2");
        assert_eq!(validate_media(&project.media), Ok(()));
        let kinds: Vec<_> = project.media.iter().map(|m| m.kind).collect();
        assert_eq!(
            kinds,
            [MediaKind::Video, MediaKind::Audio, MediaKind::Image]
        );

        let video = project.media[0].info.video.as_ref().unwrap();
        assert_eq!(video.rotation, Rotation::Cw90);
        assert_eq!(video.display_size(), (1080, 1920));
        assert_eq!(
            video.frame_rate,
            Some(crate::Rational::new(30000, 1001).unwrap())
        );
        let hdr = video.hdr.unwrap();
        assert_eq!(hdr.transfer, HdrTransfer::Hlg);
        assert_eq!(hdr.dolby_vision.unwrap().compatibility_id, 4);
        assert_eq!(project.media[0].path.relative, None);
        assert_eq!(project.media[0].file_name(), "IMG_0042.MOV");

        assert_eq!(from_json(&to_json(&project)).unwrap(), project);
    }

    /// The serialized form is the file format (ADR-0002 §2): this pins the media part of it, so a
    /// change that would need a migration fails here first.
    #[test]
    fn media_serialized_form_is_stable() {
        let mut project = Project::new("Canonical");
        project.media.push(crate::media::samples::video(1));
        let value: Value = serde_json::from_str(&to_json(&project)).unwrap();
        assert_eq!(value["schemaVersion"], CURRENT_SCHEMA_VERSION);
        let media = &value["media"][0];
        let mut keys: Vec<_> = media.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["fingerprint", "id", "info", "kind", "path"]);
        assert_eq!(media["kind"], "video");
        assert_eq!(media["path"]["relative"], Value::Null);
        assert_eq!(media["fingerprint"]["hash"], "01".repeat(16));
        let video = &media["info"]["video"];
        for key in [
            "streamIndex",
            "codec",
            "width",
            "height",
            "sampleAspect",
            "rotation",
            "frameRate",
            "pixelFormat",
            "bitDepth",
            "color",
            "hdr",
        ] {
            assert!(video.get(key).is_some(), "video.{key}");
        }
        assert_eq!(video["rotation"], "none");
        assert_eq!(
            video["sampleAspect"],
            serde_json::json!({ "num": 1, "den": 1 })
        );
        assert_eq!(media["info"]["audio"][0]["sampleRate"], 48_000);
    }

    #[test]
    fn v1_fixture_migrates_to_the_timeline_model() {
        use crate::model::{CanvasBackground, ClipSource, Rgba8};
        use crate::props::VisualProps;

        let project = from_json(include_str!("../tests/fixtures/project-v1.json")).unwrap();
        let main = project.main().unwrap();
        assert_eq!(main.name, "Video 1");
        assert_eq!(project.main_index(), Some(0));
        assert!(project.magnetic, "one clip at 0 is contiguous");
        assert_eq!(project.background, CanvasBackground::default());
        assert!(project.tracks.iter().all(|t| !t.locked));
        let clip = &main.clips[0];
        assert_eq!(
            clip.source,
            ClipSource::Color {
                color: Rgba8::MIGRATED_GRAY
            }
        );
        assert_eq!(clip.visual, Some(VisualProps::default()));
        assert_eq!((clip.link, &clip.audio), (None, &None));
    }

    #[test]
    fn v2_migration_picks_the_last_video_track_and_reorders_sections() {
        let text = |kind: &str, name: &str, clips: Value| {
            serde_json::json!({
                "id": crate::model::TrackId::new(), "kind": kind, "name": name,
                "muted": false, "hidden": false, "clips": clips,
            })
        };
        let gap = serde_json::json!([{ "id": crate::model::ClipId::new(), "name": "Late", "start": 5, "duration": 5 }]);
        let caption = serde_json::json!([{ "id": crate::model::ClipId::new(), "name": "Hi", "start": 0, "duration": 5 }]);
        let file = serde_json::json!({
            "schemaVersion": 2,
            "id": crate::model::ProjectId::new(),
            "name": "Sections",
            "settings": { "width": 1920, "height": 1080, "frameRate": { "num": 30, "den": 1 }, "sampleRate": 48000 },
            "tracks": [
                text("audio", "A1", Value::Array(vec![])),
                text("video", "V1", Value::Array(vec![])),
                text("text", "T1", caption),
                text("audio", "A2", Value::Array(vec![])),
                text("video", "V2", gap),
            ],
            "media": [],
        });
        let project = from_json(&file.to_string()).unwrap();
        let names: Vec<_> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["V1", "T1", "V2", "A1", "A2"]);
        assert_eq!(project.main().unwrap().name, "V2");
        assert!(!project.magnetic, "a clip at 5 is not contiguous from 0");
        assert_eq!(
            project.tracks[1].clips[0].source,
            crate::model::ClipSource::Text {
                content: "Hi".into()
            }
        );

        // No video track: a new empty main track is added between the sections.
        let mut file = file;
        file["tracks"] = serde_json::json!([
            text("audio", "A1", Value::Array(vec![])),
            text("text", "Video 1", Value::Array(vec![]))
        ]);
        let project = from_json(&file.to_string()).unwrap();
        let names: Vec<_> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Video 1", "Video 2", "A1"]);
        assert_eq!(project.main_index(), Some(1));
        assert!(project.magnetic);
    }

    #[test]
    fn a_v1_clip_on_an_audio_track_cannot_be_migrated() {
        let err = load(include_str!("../tests/fixtures/project-v1-audio-clip.json")).unwrap_err();
        assert!(
            matches!(err, LoadError::Migration { from: 1, .. }),
            "{err:?}"
        );
    }

    /// The v3 fixture migrates to v4 with only the speed fields added: load → save gives the v3
    /// JSON plus `speed`, `reverse` and `preservePitch`.
    #[test]
    fn loads_v3_fixture_and_migrates_it_to_v4() {
        use crate::model::{CanvasBackground, ClipSource};

        let text = include_str!("../tests/fixtures/project-v3.json");
        let loaded = load(text).unwrap();
        assert_eq!(loaded.source_version, 3);
        let project = loaded.project;
        let written: Value = serde_json::from_str(&to_json(&project)).unwrap();
        let mut expected: Value = serde_json::from_str(text).unwrap();
        expected["schemaVersion"] = Value::from(4);
        for track in expected["tracks"].as_array_mut().unwrap() {
            for clip in track["clips"].as_array_mut().unwrap() {
                if clip["source"]["kind"] == "media" {
                    clip["source"]["speed"] = Value::from(100);
                    clip["source"]["reverse"] = Value::Bool(false);
                }
                if clip["audio"].is_object() {
                    clip["audio"]["preservePitch"] = Value::Bool(true);
                }
            }
        }
        assert_eq!(written, expected);

        let names: Vec<_> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Text 1", "Video 2", "Video 1", "Audio 1", "Music"]);
        assert_eq!(project.main_index(), Some(2));
        assert!(project.magnetic && project.tracks[3].locked);
        let main = &project.tracks[2].clips[0];
        assert!(matches!(
            main.source,
            ClipSource::Media {
                speed: crate::time::Speed::NORMAL,
                reverse: false,
                ..
            }
        ));
        assert_eq!(main.link, project.tracks[3].clips[0].link);
        assert!(main.link.is_some() && main.audio.is_none());
        assert_eq!(
            main.source_time(crate::time::Time::from_seconds(2)),
            Some(crate::time::Time::from_seconds(3))
        );
        assert!(
            project.tracks[4].clips[0]
                .audio
                .as_ref()
                .unwrap()
                .preserve_pitch
        );
        let photo = project.tracks[1].clips[0].visual.as_ref().unwrap();
        assert_eq!(photo.rotation.value, 90.0);
        assert_eq!(photo.fit, crate::props::FitMode::Fill);
        assert!(matches!(project.background, CanvasBackground::Image { .. }));
    }

    /// The v4 fixture is exactly what this version writes: load → save gives the same JSON.
    #[test]
    fn loads_v4_fixture_and_writes_it_back_unchanged() {
        use crate::model::ClipSource;
        use crate::time::{Speed, Time};

        let text = include_str!("../tests/fixtures/project-v4.json");
        let loaded = load(text).unwrap();
        assert_eq!(loaded.source_version, 4);
        let project = loaded.project;
        let written: Value = serde_json::from_str(&to_json(&project)).unwrap();
        assert_eq!(written, serde_json::from_str::<Value>(text).unwrap());

        let s = Time::from_seconds;
        // A reversed 0.5x clip with its detached, linked sound.
        let main = &project.tracks[2].clips[0];
        let sound = &project.tracks[3].clips[0];
        for clip in [main, sound] {
            assert_eq!(
                (clip.speed(), clip.reversed()),
                (Speed::new(50).unwrap(), true)
            );
            assert_eq!(clip.source_end(), Some(s(4)));
            assert_eq!(clip.source_time(Time::ZERO), Some(s(4) - Time(1)));
        }
        assert_eq!(main.link, sound.link);
        // A 2x song that does not keep its pitch.
        let song = &project.tracks[4].clips[0];
        assert_eq!(song.speed(), Speed::new(200).unwrap());
        assert!(!song.audio.as_ref().unwrap().preserve_pitch);
        assert_eq!(song.source_end(), Some(s(20)));
        // A freeze frame of the main video, two seconds in.
        let freeze = &project.tracks[2].clips[2];
        assert_eq!(
            freeze.source,
            ClipSource::Freeze {
                media: main.source.media().unwrap(),
                source_time: s(2)
            }
        );
        assert_eq!(freeze.source_time(freeze.start), Some(s(2)));
        // A 3 s image (not the 5 s default).
        assert_eq!(project.tracks[1].clips[0].duration, s(3));
    }

    /// Pins the clip part of the file format (§9 example).
    #[test]
    fn clip_serialized_form_is_stable() {
        use crate::model::{Clip, ClipSource, LinkId, Rgba8};

        let media = crate::media::samples::video(1);
        let clip = Clip {
            source: ClipSource::media_at(media.id, crate::time::Time(705_600_000)),
            link: Some(LinkId(uuid::Uuid::nil())),
            ..Clip::color(
                "IMG",
                crate::time::Time(0),
                crate::time::Time(10),
                Rgba8::BLACK,
            )
        };
        let value = serde_json::to_value(&clip).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "audio", "duration", "id", "link", "name", "source", "start", "visual"
            ]
        );
        assert_eq!(
            value["source"],
            serde_json::json!({
                "kind": "media", "media": media.id, "sourceIn": 705_600_000,
                "speed": 100, "reverse": false
            })
        );
        let freeze = ClipSource::Freeze {
            media: media.id,
            source_time: crate::time::Time(1_411_200_000),
        };
        assert_eq!(
            serde_json::to_value(freeze).unwrap(),
            serde_json::json!({ "kind": "freeze", "media": media.id, "sourceTime": 1_411_200_000 })
        );
        assert_eq!(
            serde_json::to_value(crate::props::AudioProps::default()).unwrap(),
            serde_json::json!({
                "volume": { "value": 1.0 }, "fadeIn": 0, "fadeOut": 0, "preservePitch": true
            })
        );
        assert_eq!(value["audio"], Value::Null);
        assert_eq!(
            value["visual"],
            serde_json::json!({
                "position": { "value": { "x": 0.0, "y": 0.0 } },
                "scale": { "value": { "x": 1.0, "y": 1.0 } },
                "rotation": { "value": 0.0 },
                "opacity": { "value": 1.0 },
                "crop": { "value": { "left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0 } },
                "flip": { "horizontal": false, "vertical": false },
                "fit": "fit"
            })
        );
        let card = Clip::color(
            "C",
            crate::time::Time(0),
            crate::time::Time(1),
            Rgba8 {
                r: 255,
                g: 136,
                b: 0,
                a: 128,
            },
        );
        assert_eq!(
            serde_json::to_value(&card).unwrap()["source"],
            serde_json::json!({ "kind": "color", "color": "#ff880080" })
        );
        let project = Project::new("P");
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(
            value["background"],
            serde_json::json!({ "kind": "color", "color": "#000000ff" })
        );
        assert_eq!(value["magnetic"], true);
        assert_eq!(value["mainTrack"], serde_json::json!(project.main_track));
        assert_eq!(value["tracks"][0]["locked"], false);
    }

    #[test]
    fn colours_parse_strictly() {
        use crate::model::Rgba8;
        assert_eq!(
            "#FF880080".parse(),
            Ok(Rgba8 {
                r: 255,
                g: 136,
                b: 0,
                a: 128
            })
        );
        for bad in ["ff880080", "#ff8800", "#ff88008g", "#ff8800800", "#ff8800é"] {
            assert!(bad.parse::<Rgba8>().is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_newer_versions() {
        let err = from_json(r#"{"schemaVersion": 999}"#).unwrap_err();
        assert!(matches!(err, LoadError::TooNew { found: 999, .. }));
    }

    #[test]
    fn rejects_files_without_version() {
        assert!(matches!(
            from_json("{}").unwrap_err(),
            LoadError::MissingVersion
        ));
    }

    #[test]
    fn load_reports_the_source_version() {
        let loaded = load(include_str!("../tests/fixtures/project-v1.json")).unwrap();
        assert_eq!(loaded.source_version, 1);
        assert_eq!(loaded.project.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.project.validate(), Ok(()));
    }

    #[test]
    fn rejects_version_zero_and_non_numbers() {
        for text in [r#"{"schemaVersion": 0}"#, r#"{"schemaVersion": "1"}"#] {
            assert!(matches!(load(text).unwrap_err(), LoadError::MissingVersion));
        }
    }

    #[test]
    fn rejects_wrong_shapes_as_json_errors() {
        assert!(matches!(
            load(r#"{"schemaVersion": 1, "name": 5}"#).unwrap_err(),
            LoadError::Json(_)
        ));
        assert!(matches!(load("not json").unwrap_err(), LoadError::Json(_)));
    }

    #[test]
    fn rejects_invalid_projects() {
        let err = load(include_str!(
            "../tests/fixtures/project-v1-invalid-overlap.json"
        ))
        .unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::Invalid(ValidationError::ClipsOverlap { .. })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_unaligned_frame_rates() {
        let text =
            include_str!("../tests/fixtures/project-v1.json").replace("\"num\": 30", "\"num\": 11");
        assert!(matches!(from_json(&text).unwrap_err(), LoadError::Json(_)));
    }
}
