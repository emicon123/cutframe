//! Project file (de)serialization with schema versioning.
//!
//! Every format change bumps [`CURRENT_SCHEMA_VERSION`] and adds a step to [`migrate`], with a test
//! that loads a fixture written by the previous version. Reading and writing files is the shell's
//! job: this module converts to and from strings only (ADR-0002 §8).

use serde_json::Value;

use crate::model::Project;
use crate::validate::ValidationError;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
fn migrate(value: Value, version: u32) -> Value {
    // No older versions exist yet. Add steps here, e.g.:
    // if version < 2 { value = v1_to_v2(value); }
    let _ = version;
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
