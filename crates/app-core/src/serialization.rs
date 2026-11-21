//! Project file (de)serialization with schema versioning.
//!
//! Every format change bumps [`CURRENT_SCHEMA_VERSION`] and adds a step to [`migrate`], with a test
//! that loads a fixture written by the previous version.

use serde_json::Value;

use crate::model::Project;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("project file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project file has no schemaVersion")]
    MissingVersion,
    #[error("project file version {found} is newer than this app supports ({supported})")]
    TooNew { found: u64, supported: u32 },
}

pub fn to_json(project: &Project) -> String {
    serde_json::to_string_pretty(project).expect("project model is always serializable")
}

pub fn from_json(text: &str) -> Result<Project, LoadError> {
    let value: Value = serde_json::from_str(text)?;
    let value = migrate(value)?;
    Ok(serde_json::from_value(value)?)
}

/// Upgrades a project document of any supported older version to the current one.
fn migrate(value: Value) -> Result<Value, LoadError> {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or(LoadError::MissingVersion)?;
    if version > u64::from(CURRENT_SCHEMA_VERSION) {
        return Err(LoadError::TooNew {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    // No older versions exist yet. Add steps here, e.g.:
    // if version < 2 { value = v1_to_v2(value); }
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
    fn rejects_unaligned_frame_rates() {
        let text =
            include_str!("../tests/fixtures/project-v1.json").replace("\"num\": 30", "\"num\": 11");
        assert!(matches!(from_json(&text).unwrap_err(), LoadError::Json(_)));
    }
}
