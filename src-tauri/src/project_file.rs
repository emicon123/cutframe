//! Reading and atomically writing project files (ADR-0002 §8 and amendment §3-4).
//!
//! Plain blocking functions: callers run them off the main thread (`spawn_blocking`).

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use app_core::PROJECT_FILE_EXTENSION;
use app_core::serialization::{self, LoadError, Loaded};

/// How often a rename that failed with `PermissionDenied` is retried. Antivirus scanners and sync
/// clients briefly hold freshly written files open on Windows.
const RENAME_RETRIES: u32 = 3;
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Why a project file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("cannot read project file: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Load(#[from] LoadError),
}

/// Reads and loads the project at `path` (parse, migrate, validate).
pub fn read_project_file(path: &Path) -> Result<Loaded, ReadError> {
    let text = fs::read_to_string(path)?;
    Ok(serialization::load(&text)?)
}

/// Writes `json` to `path` atomically: a crash or a full disk leaves either the old file or the new
/// one, never a mix.
///
/// With `backup_of_version = Some(n)`, an existing file at `path` is first copied to
/// `<path>.v<n>.bak`, unless that backup already exists (it is never overwritten). Pass it only
/// when saving over a file that was loaded from the older schema version `n`.
///
/// On failure the temporary file is removed and the target is left as it was.
pub fn write_project_file(
    path: &Path,
    json: &str,
    backup_of_version: Option<u32>,
) -> io::Result<()> {
    if let Some(version) = backup_of_version {
        let backup = sibling(path, &format!(".v{version}.bak"));
        if path.exists() && !backup.exists() {
            fs::copy(path, &backup)?;
        }
    }

    let tmp = sibling(path, ".tmp");
    let result = write_synced(&tmp, json).and_then(|()| rename_with_retry(&tmp, path));
    if result.is_err() {
        // Best effort: the original error is what matters.
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `path` with `.<PROJECT_FILE_EXTENSION>` appended when it has no extension.
pub fn with_project_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(PROJECT_FILE_EXTENSION)
    }
}

/// `path` with `suffix` appended to its file name, in the same directory (so a rename stays on
/// the same volume).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn write_synced(path: &Path, contents: &str) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// Renames `from` over `to`, replacing it (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on
/// Windows, `rename(2)` elsewhere).
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied && attempt < RENAME_RETRIES =>
            {
                attempt += 1;
                thread::sleep(RENAME_RETRY_DELAY);
            }
            result => return result,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Instant;

    use app_core::{Clip, Project, Rgba8, Time, Track, TrackKind};

    use super::*;

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_a_new_file_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cutframe");
        write_project_file(&path, "new", None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(files_in(dir.path()), ["a.cutframe"]);
    }

    #[test]
    fn replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cutframe");
        fs::write(&path, "old contents that are longer").unwrap();
        write_project_file(&path, "new", None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(files_in(dir.path()), ["a.cutframe"]);
    }

    #[test]
    fn a_failed_write_leaves_the_target_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        // A directory with files in it cannot be replaced by a file.
        let target = dir.path().join("a.cutframe");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("inside"), "keep").unwrap();

        assert!(write_project_file(&target, "new", None).is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_to_string(target.join("inside")).unwrap(), "keep");
        assert_eq!(files_in(dir.path()), ["a.cutframe"], "no .tmp left behind");
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("a.cutframe");
        let err = write_project_file(&path, "new", None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn backs_up_an_older_version_once_and_never_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cutframe");
        fs::write(&path, "version 1").unwrap();

        write_project_file(&path, "version 2, first save", Some(1)).unwrap();
        let backup = dir.path().join("a.cutframe.v1.bak");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "version 1");
        assert_eq!(fs::read_to_string(&path).unwrap(), "version 2, first save");

        write_project_file(&path, "version 2, second save", Some(1)).unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), "version 1");
        assert_eq!(files_in(dir.path()), ["a.cutframe", "a.cutframe.v1.bak"]);
    }

    #[test]
    fn no_backup_without_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cutframe");
        write_project_file(&path, "new", Some(1)).unwrap();
        assert_eq!(files_in(dir.path()), ["a.cutframe"]);
    }

    #[test]
    fn appends_the_extension_only_when_missing() {
        assert_eq!(
            with_project_extension(PathBuf::from("dir/My video")),
            PathBuf::from("dir/My video.cutframe")
        );
        assert_eq!(
            with_project_extension(PathBuf::from("dir/a.cutframe")),
            PathBuf::from("dir/a.cutframe")
        );
        assert_eq!(
            with_project_extension(PathBuf::from("dir/a.json")),
            PathBuf::from("dir/a.json")
        );
    }

    #[test]
    fn saved_files_load_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cutframe");
        let project = Project::new("Round trip");
        write_project_file(&path, &serialization::to_json(&project), None).unwrap();
        let loaded = read_project_file(&path).unwrap();
        assert_eq!(loaded.project, project);
    }

    #[test]
    fn read_errors_keep_their_cause() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read_project_file(&dir.path().join("missing.cutframe")).unwrap_err();
        assert!(matches!(&missing, ReadError::Io(e) if e.kind() == io::ErrorKind::NotFound));

        let path = dir.path().join("bad.cutframe");
        fs::write(&path, "{}").unwrap();
        assert!(matches!(
            read_project_file(&path).unwrap_err(),
            ReadError::Load(LoadError::MissingVersion)
        ));
    }

    /// Design budget (project-lifecycle §8): save and open of a 1,000-clip, 20-track project take
    /// under 50 ms each. Run with `cargo test --release -p cutframe -- --ignored`.
    /// A 1,000-clip, 20-track project for timing tests.
    pub(crate) fn large_project() -> Project {
        let mut project = Project::new("Large");
        project.magnetic = false;
        project.tracks = (0..20)
            .map(|t| {
                let mut track = Track::new(TrackKind::Video, format!("Track {t}"));
                track.clips = (0..50)
                    .map(|c| {
                        Clip::color(
                            format!("Clip {t}.{c}"),
                            Time::from_seconds(c * 10),
                            Time::from_seconds(5),
                            Rgba8::rgb(0x20, 0x40, 0x80),
                        )
                    })
                    .collect();
                track
            })
            .collect();
        // The bottom track is the main track; the others are overlays.
        project.main_track = project.tracks[19].id;
        project
    }

    #[test]
    #[ignore = "timing test; run in release mode"]
    fn large_project_saves_and_opens_within_budget() {
        let project = large_project();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.cutframe");

        let started = Instant::now();
        write_project_file(&path, &serialization::to_json(&project), None).unwrap();
        let save = started.elapsed();
        let started = Instant::now();
        let loaded = read_project_file(&path).unwrap();
        let open = started.elapsed();

        assert_eq!(loaded.project, project);
        assert!(save < Duration::from_millis(50), "save took {save:?}");
        assert!(open < Duration::from_millis(50), "open took {open:?}");
    }
}
