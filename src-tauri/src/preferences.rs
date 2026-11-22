//! User preferences (`preferences.json` in the app config directory) and the cache folder
//! (design `preferences.md` §4).
//!
//! Not the project model, so not `app-core` and not undoable. One struct is both the file format
//! and the IPC type: any serialized change bumps [`CURRENT_PREFERENCES_VERSION`], adds a step to
//! [`migrate`] and a fixture `tests/fixtures/preferences-v<N>.json` (kept forever).
//!
//! Plain blocking functions without Tauri: callers run file work off the main thread.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use app_core::{FLICKS_PER_SECOND, MAX_IPC_FLICKS, Time};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::project_file;

pub const CURRENT_PREFERENCES_VERSION: u32 = 1;

/// File name inside the app config directory.
pub const FILE_NAME: &str = "preferences.json";

/// The only folder the app deletes in, inside the configured cache folder. A user who picks
/// e.g. `D:\Videos` as the cache folder can never lose files outside it.
pub const CACHE_SUBFOLDER: &str = "cutframe-cache";

const DEFAULT_AUTOSAVE_SECONDS: i64 = 60;
const AUTOSAVE_RANGE_SECONDS: (i64, i64) = (15, 3_600);
/// 0.1 s in flicks.
const TENTH_SECOND: i64 = FLICKS_PER_SECOND / 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// Always the current version when sent or written; `setPreferences` rejects any other.
    pub schema_version: u32,
    /// Flicks between autosaves, a whole number of seconds in 15 s ..= 3,600 s; `None` = off.
    pub autosave_interval: Option<Time>,
    /// Flicks, 0.1 s ..= 600 s. Used when an image is added to the timeline.
    pub default_image_duration: Time,
    /// Flicks, 0.1 s ..= 10 s. Used when a transition is added.
    pub default_transition_duration: Time,
    pub language: UiLanguage,
    /// Absolute folder that holds the cache; `None` = the app's cache directory. The app only
    /// deletes inside its `cutframe-cache` subfolder.
    pub cache_dir: Option<String>,
}

/// UI language. The UI picks its translation table by this key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UiLanguage {
    #[default]
    En,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_PREFERENCES_VERSION,
            autosave_interval: Some(Time::from_seconds(DEFAULT_AUTOSAVE_SECONDS)),
            default_image_duration: Time::from_seconds(5),
            default_transition_duration: Time::from_flicks(FLICKS_PER_SECOND / 2),
            language: UiLanguage::En,
            cache_dir: None,
        }
    }
}

/// Why preferences were rejected. The message names the field (camelCase, as in the file).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreferencesError {
    #[error("schemaVersion {0} is not the current version {CURRENT_PREFERENCES_VERSION}")]
    WrongVersion(u32),
    #[error("{0} is out of range")]
    OutOfRange(&'static str),
    #[error("autosaveInterval must be a whole number of seconds")]
    NotWholeSeconds,
    #[error("cacheDir must be an absolute path")]
    RelativeCacheDir,
}

impl Preferences {
    /// Checks every field (design §4). `schemaVersion` is checked by the caller.
    pub fn validate(&self) -> Result<(), PreferencesError> {
        check_autosave(self.autosave_interval)?;
        check_image_duration(self.default_image_duration)?;
        check_transition_duration(self.default_transition_duration)?;
        check_cache_dir(self.cache_dir.as_deref())
    }

    /// Replaces each invalid field with its default (logged), so a bad value never stops the
    /// app. Returns the names of the replaced fields.
    pub fn sanitize(&mut self) -> Vec<&'static str> {
        let defaults = Self::default();
        let mut replaced = Vec::new();
        let mut fix = |field: &'static str, bad: bool| {
            if bad {
                tracing::warn!(field, "invalid preference replaced with its default");
                replaced.push(field);
            }
            bad
        };
        if fix(
            "autosaveInterval",
            check_autosave(self.autosave_interval).is_err(),
        ) {
            self.autosave_interval = defaults.autosave_interval;
        }
        if fix(
            "defaultImageDuration",
            check_image_duration(self.default_image_duration).is_err(),
        ) {
            self.default_image_duration = defaults.default_image_duration;
        }
        if fix(
            "defaultTransitionDuration",
            check_transition_duration(self.default_transition_duration).is_err(),
        ) {
            self.default_transition_duration = defaults.default_transition_duration;
        }
        if fix(
            "cacheDir",
            check_cache_dir(self.cache_dir.as_deref()).is_err(),
        ) {
            self.cache_dir = defaults.cache_dir;
        }
        self.schema_version = CURRENT_PREFERENCES_VERSION;
        replaced
    }

    /// The autosave interval as a [`Duration`]; `None` = off. Valid preferences only.
    pub fn autosave_duration(&self) -> Option<Duration> {
        self.autosave_interval.map(to_duration)
    }

    /// The folder the app caches into (and the only one it deletes in):
    /// `<cache_dir or app_cache_dir>/cutframe-cache`.
    pub fn cache_root(&self, app_cache_dir: &Path) -> PathBuf {
        self.cache_dir
            .as_deref()
            .map_or(app_cache_dir, Path::new)
            .join(CACHE_SUBFOLDER)
    }
}

fn in_range(field: &'static str, time: Time, min: i64, max: i64) -> Result<(), PreferencesError> {
    // The ranges are far inside ±2^53 (ADR-0003 §3); the explicit bound documents it.
    let flicks = time.flicks();
    if flicks.unsigned_abs() > MAX_IPC_FLICKS.unsigned_abs() || !(min..=max).contains(&flicks) {
        return Err(PreferencesError::OutOfRange(field));
    }
    Ok(())
}

fn check_autosave(interval: Option<Time>) -> Result<(), PreferencesError> {
    let Some(interval) = interval else {
        return Ok(());
    };
    let (min, max) = AUTOSAVE_RANGE_SECONDS;
    in_range(
        "autosaveInterval",
        interval,
        min * FLICKS_PER_SECOND,
        max * FLICKS_PER_SECOND,
    )?;
    if interval.flicks() % FLICKS_PER_SECOND != 0 {
        return Err(PreferencesError::NotWholeSeconds);
    }
    Ok(())
}

fn check_image_duration(duration: Time) -> Result<(), PreferencesError> {
    in_range(
        "defaultImageDuration",
        duration,
        TENTH_SECOND,
        600 * FLICKS_PER_SECOND,
    )
}

fn check_transition_duration(duration: Time) -> Result<(), PreferencesError> {
    in_range(
        "defaultTransitionDuration",
        duration,
        TENTH_SECOND,
        10 * FLICKS_PER_SECOND,
    )
}

fn check_cache_dir(dir: Option<&str>) -> Result<(), PreferencesError> {
    match dir {
        Some(dir) if !Path::new(dir).is_absolute() => Err(PreferencesError::RelativeCacheDir),
        _ => Ok(()),
    }
}

/// `time` (non-negative flicks) as a [`Duration`], exact to the nanosecond.
fn to_duration(time: Time) -> Duration {
    let flicks = u128::try_from(time.flicks()).unwrap_or(0);
    let nanos = flicks * 1_000_000_000 / FLICKS_PER_SECOND as u128;
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("preferences file is not valid JSON or has the wrong shape: {0}")]
    Json(#[from] serde_json::Error),
    /// `schemaVersion` is missing, not a number, or 0.
    #[error("preferences file has no valid schemaVersion")]
    MissingVersion,
    #[error("preferences file version {found} is newer than this app supports")]
    TooNew { found: u64 },
}

/// Preferences read from a file, with the version it was written in.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// Migrated and sanitized.
    pub preferences: Preferences,
    pub source_version: u32,
}

/// The load pipeline: parse → `schemaVersion` → reject newer → migrate step by step →
/// deserialize (a missing field takes its default) → sanitize.
pub fn load(text: &str) -> Result<Loaded, LoadError> {
    let value: Value = serde_json::from_str(text)?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .filter(|&v| v >= 1)
        .ok_or(LoadError::MissingVersion)?;
    if version > u64::from(CURRENT_PREFERENCES_VERSION) {
        return Err(LoadError::TooNew { found: version });
    }
    // Lossless: at most CURRENT_PREFERENCES_VERSION.
    let source_version = version as u32;
    let mut preferences: Preferences =
        serde_json::from_value(run_steps(value, source_version, &MIGRATIONS))?;
    preferences.sanitize();
    Ok(Loaded {
        preferences,
        source_version,
    })
}

/// One migration step: turns a document of version `n` into version `n + 1` (and sets
/// `schemaVersion`).
type Step = fn(Value) -> Value;

/// `MIGRATIONS[i]` upgrades version `i + 1` to `i + 2`. Empty at v1; v2 adds `v1_to_v2` here.
const MIGRATIONS: [Step; CURRENT_PREFERENCES_VERSION as usize - 1] = [];

/// Applies `steps` to a document of version `from` (1-based), in order.
fn run_steps(mut value: Value, from: u32, steps: &[Step]) -> Value {
    for step in steps.iter().skip(from.saturating_sub(1) as usize) {
        value = step(value);
    }
    value
}

/// Preferences in effect at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub preferences: Preferences,
    /// Pass to the first write: the file came from another version (older or newer) and is kept
    /// as `preferences.json.v<n>.bak`.
    pub backup_of_version: Option<u32>,
}

/// Reads the preferences at `path` at startup. Never fails: a missing or unreadable file gives the
/// defaults (nothing is written until the user saves).
pub fn read_startup(path: &Path) -> Startup {
    let defaults = |backup_of_version| Startup {
        preferences: Preferences::default(),
        backup_of_version,
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return defaults(None),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "cannot read preferences; using defaults");
            return defaults(None);
        }
    };
    match load(&text) {
        Ok(loaded) => Startup {
            backup_of_version: (loaded.source_version < CURRENT_PREFERENCES_VERSION)
                .then_some(loaded.source_version),
            preferences: loaded.preferences,
        },
        Err(LoadError::TooNew { found }) => {
            tracing::warn!(
                found,
                "preferences are from a newer version; using defaults"
            );
            defaults(Some(u32::try_from(found).unwrap_or(u32::MAX)))
        }
        Err(err) => {
            tracing::warn!(%err, "invalid preferences file; using defaults");
            defaults(None)
        }
    }
}

/// Writes `preferences` atomically (tmp + rename). With `backup_of_version = Some(n)` the existing
/// file is first copied to `preferences.json.v<n>.bak`, unless that backup exists.
pub fn write(
    path: &Path,
    preferences: &Preferences,
    backup_of_version: Option<u32>,
) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(preferences).expect("preferences always serialize");
    project_file::write_project_file(path, &json, backup_of_version)
}

/// Size and file count of a cache root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    pub size_bytes: u64,
    pub files: u32,
}

/// Walks `root` (missing = empty). Symbolic links are counted as files, never followed.
pub fn cache_info(root: &Path) -> io::Result<CacheStats> {
    let mut stats = CacheStats::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        for entry in entries {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                stats.size_bytes += metadata.len();
                stats.files = stats.files.saturating_add(1);
            }
        }
    }
    Ok(stats)
}

/// Deletes everything inside `root`, keeping `root` itself. Only ever pass a path from
/// [`Preferences::cache_root`], which ends in [`CACHE_SUBFOLDER`].
pub fn clear_cache(root: &Path) -> io::Result<()> {
    if root.file_name() != Some(CACHE_SUBFOLDER.as_ref()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to clear {}: not a cache root", root.display()),
        ));
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let path = entry?.path();
        // `remove_dir_all` does not follow symbolic links; a link itself is removed as a file.
        if path.symlink_metadata()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute_dir() -> String {
        std::env::temp_dir().to_string_lossy().into_owned()
    }

    fn secs(seconds: i64) -> Time {
        Time::from_seconds(seconds)
    }

    #[test]
    fn defaults_are_valid() {
        let defaults = Preferences::default();
        assert_eq!(defaults.validate(), Ok(()));
        assert_eq!(defaults.autosave_duration(), Some(Duration::from_secs(60)));
        assert_eq!(defaults.default_image_duration, secs(5));
        assert_eq!(defaults.default_transition_duration.flicks(), 352_800_000);
    }

    #[test]
    fn range_bounds_are_inclusive() {
        let tenth = Time::from_flicks(TENTH_SECOND);
        let just_below = Time::from_flicks(TENTH_SECOND - 1);
        type Setter = fn(&mut Preferences, Time);
        let cases: [(Setter, Time, bool); 12] = [
            (|p, t| p.autosave_interval = Some(t), secs(15), true),
            (|p, t| p.autosave_interval = Some(t), secs(14), false),
            (|p, t| p.autosave_interval = Some(t), secs(3_600), true),
            (|p, t| p.autosave_interval = Some(t), secs(3_601), false),
            (|p, t| p.default_image_duration = t, tenth, true),
            (|p, t| p.default_image_duration = t, just_below, false),
            (|p, t| p.default_image_duration = t, secs(600), true),
            (|p, t| p.default_image_duration = t, secs(601), false),
            (|p, t| p.default_transition_duration = t, tenth, true),
            (|p, t| p.default_transition_duration = t, just_below, false),
            (|p, t| p.default_transition_duration = t, secs(10), true),
            (|p, t| p.default_transition_duration = t, secs(11), false),
        ];
        for (set, time, ok) in cases {
            let mut prefs = Preferences::default();
            set(&mut prefs, time);
            assert_eq!(prefs.validate().is_ok(), ok, "{time:?}: {prefs:?}");
        }
    }

    #[test]
    fn autosave_off_is_valid_and_fractions_of_a_second_are_not() {
        let mut prefs = Preferences {
            autosave_interval: None,
            ..Preferences::default()
        };
        assert_eq!(prefs.validate(), Ok(()));
        assert_eq!(prefs.autosave_duration(), None);
        prefs.autosave_interval = Some(Time::from_flicks(30 * FLICKS_PER_SECOND + 1));
        assert_eq!(prefs.validate(), Err(PreferencesError::NotWholeSeconds));
    }

    #[test]
    fn times_beyond_the_ipc_range_are_rejected() {
        for flicks in [MAX_IPC_FLICKS + 1, i64::MAX, i64::MIN, -1] {
            let prefs = Preferences {
                default_image_duration: Time::from_flicks(flicks),
                ..Preferences::default()
            };
            assert_eq!(
                prefs.validate(),
                Err(PreferencesError::OutOfRange("defaultImageDuration"))
            );
        }
    }

    #[test]
    fn cache_dir_must_be_absolute() {
        let mut prefs = Preferences {
            cache_dir: Some("relative/cache".into()),
            ..Preferences::default()
        };
        assert_eq!(prefs.validate(), Err(PreferencesError::RelativeCacheDir));
        prefs.cache_dir = Some(absolute_dir());
        assert_eq!(prefs.validate(), Ok(()));
    }

    #[test]
    fn sanitize_replaces_only_the_bad_fields() {
        let mut prefs = Preferences {
            autosave_interval: Some(secs(120)),
            default_image_duration: secs(9_999),
            default_transition_duration: secs(2),
            cache_dir: Some("relative".into()),
            ..Preferences::default()
        };
        assert_eq!(prefs.sanitize(), ["defaultImageDuration", "cacheDir"]);
        assert_eq!(
            prefs,
            Preferences {
                autosave_interval: Some(secs(120)),
                default_transition_duration: secs(2),
                ..Preferences::default()
            }
        );
        assert!(prefs.sanitize().is_empty());
    }

    fn fixtures() -> Vec<(u32, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        (1..=CURRENT_PREFERENCES_VERSION)
            .map(|v| {
                let path = dir.join(format!("preferences-v{v}.json"));
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
                (v, text)
            })
            .collect()
    }

    /// Every fixture, one per version and kept forever, loads into the current struct. The v1
    /// fixture has non-default values, so a field that silently fell back would fail.
    #[test]
    fn every_fixture_loads() {
        for (version, text) in fixtures() {
            let loaded = load(&text).unwrap_or_else(|err| panic!("v{version}: {err}"));
            assert_eq!(loaded.source_version, version);
            assert_eq!(
                loaded.preferences.schema_version,
                CURRENT_PREFERENCES_VERSION
            );
            assert_eq!(loaded.preferences.validate(), Ok(()));
        }
        let v1 = load(&fixtures()[0].1).unwrap().preferences;
        assert_eq!(
            v1,
            Preferences {
                schema_version: 1,
                autosave_interval: Some(secs(120)),
                default_image_duration: secs(3),
                default_transition_duration: Time::from_flicks(FLICKS_PER_SECOND),
                language: UiLanguage::En,
                cache_dir: None,
            }
        );
    }

    #[test]
    fn current_preferences_round_trip() {
        let prefs = Preferences {
            autosave_interval: None,
            cache_dir: Some(absolute_dir()),
            ..Preferences::default()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert_eq!(load(&json).unwrap().preferences, prefs);
    }

    #[test]
    fn serializes_camel_case_keys_and_null_for_off() {
        let value = serde_json::to_value(Preferences {
            autosave_interval: None,
            ..Preferences::default()
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "schemaVersion": 1,
                "autosaveInterval": null,
                "defaultImageDuration": 3_528_000_000_i64,
                "defaultTransitionDuration": 352_800_000,
                "language": "en",
                "cacheDir": null,
            })
        );
    }

    #[test]
    fn missing_fields_take_defaults_and_bad_values_are_sanitized() {
        let loaded = load(r#"{ "schemaVersion": 1, "defaultImageDuration": -5 }"#).unwrap();
        assert_eq!(loaded.preferences, Preferences::default());
        // An explicit null turns autosave off; a missing field keeps the default.
        let off = load(r#"{ "schemaVersion": 1, "autosaveInterval": null }"#).unwrap();
        assert_eq!(off.preferences.autosave_interval, None);
    }

    #[test]
    fn rejects_newer_missing_and_broken_files() {
        assert!(matches!(
            load(r#"{ "schemaVersion": 2 }"#),
            Err(LoadError::TooNew { found: 2 })
        ));
        for text in [
            "{}",
            r#"{ "schemaVersion": 0 }"#,
            r#"{ "schemaVersion": "1" }"#,
        ] {
            assert!(
                matches!(load(text), Err(LoadError::MissingVersion)),
                "{text}"
            );
        }
        assert!(matches!(load("not json"), Err(LoadError::Json(_))));
        assert!(matches!(
            load(r#"{ "schemaVersion": 1, "language": "xx" }"#),
            Err(LoadError::Json(_))
        ));
    }

    #[test]
    fn steps_run_in_order_from_the_source_version() {
        fn v1_to_v2(mut value: Value) -> Value {
            value["schemaVersion"] = 2.into();
            value["log"] = format!("{}a", value["log"].as_str().unwrap_or("")).into();
            value
        }
        fn v2_to_v3(mut value: Value) -> Value {
            value["schemaVersion"] = 3.into();
            value["log"] = format!("{}b", value["log"].as_str().unwrap_or("")).into();
            value
        }
        // A test-only chain for a pretend current version 3.
        let steps: [Step; 2] = [v1_to_v2, v2_to_v3];
        let ran = |from| run_steps(serde_json::json!({}), from, &steps);
        assert_eq!(ran(1)["log"], "ab");
        assert_eq!(ran(1)["schemaVersion"], 3);
        assert_eq!(ran(2)["log"], "b");
        assert_eq!(ran(3), serde_json::json!({}), "current: nothing to do");
    }

    #[test]
    fn startup_falls_back_to_defaults_and_marks_foreign_versions_for_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(
            read_startup(&path),
            Startup {
                preferences: Preferences::default(),
                backup_of_version: None,
            },
            "missing file"
        );
        assert!(!path.exists(), "nothing is written at startup");

        fs::write(&path, "{ broken").unwrap();
        assert_eq!(read_startup(&path).backup_of_version, None);

        fs::write(&path, r#"{ "schemaVersion": 7, "autosaveInterval": 30 }"#).unwrap();
        assert_eq!(
            read_startup(&path),
            Startup {
                preferences: Preferences::default(),
                backup_of_version: Some(7),
            }
        );

        fs::write(&path, &fixtures()[0].1).unwrap();
        let startup = read_startup(&path);
        assert_eq!(startup.backup_of_version, None, "current version");
        assert_eq!(startup.preferences.autosave_interval, Some(secs(120)));
    }

    #[test]
    fn the_first_write_after_a_foreign_version_keeps_one_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join(FILE_NAME);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let newer = r#"{ "schemaVersion": 7 }"#;
        fs::write(&path, newer).unwrap();
        let startup = read_startup(&path);

        write(&path, &startup.preferences, startup.backup_of_version).unwrap();
        let backup = path.with_file_name("preferences.json.v7.bak");
        assert_eq!(fs::read_to_string(&backup).unwrap(), newer);
        assert_eq!(
            read_startup(&path),
            Startup {
                preferences: Preferences::default(),
                backup_of_version: None,
            }
        );

        // The backup is never overwritten.
        fs::write(&path, r#"{ "schemaVersion": 7, "other": true }"#).unwrap();
        write(&path, &Preferences::default(), Some(7)).unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), newer);
    }

    #[test]
    fn write_creates_the_config_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join(FILE_NAME);
        write(&path, &Preferences::default(), None).unwrap();
        assert_eq!(read_startup(&path).preferences, Preferences::default());
    }

    #[test]
    fn cache_root_is_a_subfolder_of_the_chosen_folder() {
        let app_cache = Path::new("/app/cache");
        assert_eq!(
            Preferences::default().cache_root(app_cache),
            app_cache.join(CACHE_SUBFOLDER)
        );
        let chosen = absolute_dir();
        let prefs = Preferences {
            cache_dir: Some(chosen.clone()),
            ..Preferences::default()
        };
        assert_eq!(
            prefs.cache_root(app_cache),
            Path::new(&chosen).join(CACHE_SUBFOLDER)
        );
    }

    #[test]
    fn cache_info_counts_nested_files_and_clear_keeps_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(CACHE_SUBFOLDER);
        assert_eq!(cache_info(&root).unwrap(), CacheStats::default(), "missing");
        fs::create_dir_all(root.join("thumbnails/ab")).unwrap();
        fs::write(root.join("thumbnails/ab/1.jpg"), [0u8; 10]).unwrap();
        fs::write(root.join("thumbnails/2.jpg"), [0u8; 20]).unwrap();
        fs::write(root.join("index.db"), [0u8; 5]).unwrap();
        // The user's own files next to the cache root.
        fs::write(dir.path().join("holiday.mp4"), [1u8; 7]).unwrap();
        fs::create_dir(dir.path().join("Projects")).unwrap();

        assert_eq!(
            cache_info(&root).unwrap(),
            CacheStats {
                size_bytes: 35,
                files: 3,
            }
        );

        clear_cache(&root).unwrap();
        assert!(root.is_dir(), "the root itself stays");
        assert_eq!(cache_info(&root).unwrap(), CacheStats::default());
        assert!(dir.path().join("holiday.mp4").is_file());
        assert!(dir.path().join("Projects").is_dir());
    }

    #[test]
    fn clear_refuses_anything_but_a_cache_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "x").unwrap();
        assert_eq!(
            clear_cache(dir.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(dir.path().join("keep.txt").is_file());
        clear_cache(&dir.path().join(CACHE_SUBFOLDER)).expect("missing root is fine");
    }

    #[test]
    fn durations_convert_exactly() {
        assert_eq!(to_duration(secs(15)), Duration::from_secs(15));
        assert_eq!(
            to_duration(Time::from_flicks(TENTH_SECOND)),
            Duration::from_millis(100)
        );
    }
}
