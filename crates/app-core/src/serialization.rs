//! Project file (de)serialization with schema versioning.
//!
//! Every format change bumps [`CURRENT_SCHEMA_VERSION`] and adds a step to [`migrate`], with a test
//! that loads a fixture written by the previous version. Reading and writing files is the shell's
//! job: this module converts to and from strings only (ADR-0002 §8).

use serde_json::Value;

use crate::model::Project;
use crate::validate::ValidationError;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

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
    let value = migrate(value, source_version);
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
fn migrate(mut value: Value, version: u32) -> Value {
    if version < 2 {
        value = v1_to_v2(value);
    }
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
        assert_eq!(value["schemaVersion"], 2);
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
