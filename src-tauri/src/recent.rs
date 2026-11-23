//! Recent projects for the home screen (`recent.json` in the app data directory) and the file
//! actions on them: rename, duplicate, move to the Recycle Bin (design `home-screen.md` §4-5).
//!
//! Not the project model, so not undoable. The file has its own [`CURRENT_RECENT_VERSION`]; any
//! serialized change bumps it, adds a step to [`MIGRATIONS`] and a fixture
//! `tests/fixtures/recent-v<N>.json` (kept forever).
//!
//! Plain blocking functions without Tauri: callers hold the recent lock and run them off the main
//! thread (`spawn_blocking`).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use app_core::serialization::{self, CURRENT_SCHEMA_VERSION};
use app_core::{PROJECT_FILE_EXTENSION, Project, ProjectId, Time};
use engine::media::cache::valid_hash;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ipc::IpcError;
use crate::project_file;

pub const CURRENT_RECENT_VERSION: u32 = 1;

/// At most this many entries; the oldest `openedAtMs` goes first.
pub const RECENT_CAP: usize = 30;

/// File name inside the app data directory.
pub const FILE_NAME: &str = "recent.json";

/// Folder inside the app data directory with one `<hash>.jpg` per cover. Outside the cache root, so
/// **Clear cache** and LRU eviction never blank the home screen.
pub const COVERS_DIR: &str = "covers";

/// The backup version used for a `recent.json` that is not a recent list at all (not JSON, no
/// `schemaVersion`, wrong shape): it is kept once as `recent.json.v0.bak`.
const BROKEN_FILE_VERSION: u32 = 0;

/// Stem of a project file whose name has nothing usable left (same as the UI's `defaultFileName`).
const UNTITLED_STEM: &str = "Untitled";

/// The file format: the version and the entries, newest first.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentFile {
    schema_version: u32,
    entries: Vec<RecentEntry>,
}

/// One project the app opened or saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    /// Absolute path; the key, compared case-insensitively (Windows, see [`same_path`]).
    pub path: String,
    /// The project name at the last open or save.
    pub name: String,
    /// `Project::duration()` at that time, flicks.
    pub duration: Time,
    /// Unix milliseconds of the last open, save or duplicate by this app.
    pub opened_at_ms: i64,
    /// `cover_hash(project)`: 32 lowercase hex characters.
    #[serde(default)]
    pub cover: Option<String>,
}

impl RecentEntry {
    /// The entry for `project`, just opened from or saved to `path`.
    pub fn new(project: &Project, path: &Path) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            name: project.name.clone(),
            duration: project.duration(),
            opened_at_ms: now_ms(),
            cover: app_core::cover_hash(project).map(str::to_owned),
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("recent projects file is not valid JSON or has the wrong shape: {0}")]
    Json(#[from] serde_json::Error),
    /// `schemaVersion` is missing, not a number, or 0.
    #[error("recent projects file has no valid schemaVersion")]
    MissingVersion,
    #[error("recent projects file version {found} is newer than this app supports")]
    TooNew { found: u64 },
}

/// The load pipeline: parse → `schemaVersion` → reject newer → migrate step by step →
/// deserialize. Returns the entries (at most [`RECENT_CAP`]) and the version the file was written in.
pub fn load(text: &str) -> Result<(Vec<RecentEntry>, u32), LoadError> {
    let value: Value = serde_json::from_str(text)?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .filter(|&v| v >= 1)
        .ok_or(LoadError::MissingVersion)?;
    if version > u64::from(CURRENT_RECENT_VERSION) {
        return Err(LoadError::TooNew { found: version });
    }
    // Lossless: at most CURRENT_RECENT_VERSION.
    let source_version = version as u32;
    let file: RecentFile = serde_json::from_value(migrate(value, source_version))?;
    let mut entries = file.entries;
    entries.truncate(RECENT_CAP);
    Ok((entries, source_version))
}

/// One migration step: turns a document of version `n` into version `n + 1` (and sets
/// `schemaVersion`).
type Step = fn(Value) -> Value;

/// `MIGRATIONS[i]` upgrades version `i + 1` to `i + 2`. Empty at v1; v2 adds `v1_to_v2` here.
const MIGRATIONS: [Step; CURRENT_RECENT_VERSION as usize - 1] = [];

/// Upgrades a document of version `from` (1-based) to the current version.
fn migrate(mut value: Value, from: u32) -> Value {
    for step in MIGRATIONS.iter().skip(from.saturating_sub(1) as usize) {
        value = step(value);
    }
    value
}

/// Puts `entry` first, dropping any entry with the same path, and keeps at most [`RECENT_CAP`].
pub fn upsert(entries: &mut Vec<RecentEntry>, entry: RecentEntry) {
    entries.retain(|e| !same_path(&e.path, &entry.path));
    entries.insert(0, entry);
    entries.truncate(RECENT_CAP);
}

/// Removes the entry for `path`. Returns whether there was one.
pub fn remove(entries: &mut Vec<RecentEntry>, path: &str) -> bool {
    let before = entries.len();
    entries.retain(|e| !same_path(&e.path, path));
    entries.len() != before
}

/// Whether two paths name the same file. Case-insensitive, as Windows file systems are.
pub fn same_path(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// `<dir>/<stem>.cutframe`, where the stem is `name` with characters Windows forbids in file names
/// (`<>:"/\|?*` and control characters) replaced by `_`, trimmed, without trailing dots and
/// spaces; `Untitled` when nothing is left. The same rule as the UI's `defaultFileName`.
pub fn project_path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{PROJECT_FILE_EXTENSION}", file_stem(name)))
}

fn file_stem(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = replaced.trim().trim_end_matches(['.', ' ']);
    if stem.is_empty() {
        UNTITLED_STEM.to_owned()
    } else {
        stem.to_owned()
    }
}

/// [`project_path_for`] if nothing `exists` there, else the first free `<stem> (2).cutframe`,
/// `<stem> (3).cutframe`, ...
pub fn free_project_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = project_path_for(dir, name);
    if !exists(&first) {
        return first;
    }
    let stem = file_stem(name);
    (2u32..)
        .map(|n| dir.join(format!("{stem} ({n}).{PROJECT_FILE_EXTENSION}")))
        .find(|path| !exists(path))
        .expect("some numbered name is free")
}

/// `recent.json` and the covers folder, both in the app data directory.
#[derive(Debug, Clone)]
pub struct RecentStore {
    file: PathBuf,
    covers: PathBuf,
}

/// The list as read from disk.
#[derive(Debug, Default)]
struct Stored {
    entries: Vec<RecentEntry>,
    /// The file is broken, newer or older: the next write keeps it once as `.v<n>.bak`.
    backup_of_version: Option<u32>,
}

/// One entry of [`RecentStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub entry: RecentEntry,
    /// The file's modification time in Unix milliseconds; `None` when it is missing or unreachable.
    pub modified_ms: Option<i64>,
}

impl RecentStore {
    /// The store in `app_data_dir`.
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            file: app_data_dir.join(FILE_NAME),
            covers: app_data_dir.join(COVERS_DIR),
        }
    }

    /// The covers folder the `media://cover/<hash>` route serves from.
    pub fn covers_dir(&self) -> &Path {
        &self.covers
    }

    /// Reads the list. Never fails: a missing file is an empty list; a broken, newer or unreadable
    /// file is an empty list too (logged), and the next write backs up a broken or newer one.
    fn read(&self) -> Stored {
        let text = match fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Stored::default(),
            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                tracing::warn!(%err, "recent projects file is not UTF-8; starting empty");
                return broken();
            }
            Err(err) => {
                tracing::warn!(%err, path = %self.file.display(), "cannot read recent projects");
                return Stored::default();
            }
        };
        match load(&text) {
            Ok((entries, source_version)) => Stored {
                entries,
                backup_of_version: (source_version < CURRENT_RECENT_VERSION)
                    .then_some(source_version),
            },
            Err(LoadError::TooNew { found }) => {
                tracing::warn!(
                    found,
                    "recent projects are from a newer version; starting empty"
                );
                Stored {
                    entries: Vec::new(),
                    backup_of_version: Some(u32::try_from(found).unwrap_or(u32::MAX)),
                }
            }
            Err(err) => {
                tracing::warn!(%err, "invalid recent projects file; starting empty");
                broken()
            }
        }
    }

    /// Writes `entries` atomically in the current version, keeping a backup when asked.
    fn write(&self, entries: &[RecentEntry], backup_of_version: Option<u32>) -> io::Result<()> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = RecentFile {
            schema_version: CURRENT_RECENT_VERSION,
            entries: entries.to_vec(),
        };
        let json = serde_json::to_string_pretty(&file).expect("recent projects always serialize");
        project_file::write_project_file(&self.file, &json, backup_of_version)
    }

    /// Read, change, write. `change` returns whether it changed anything; nothing is written if
    /// not.
    fn modify(&self, change: impl FnOnce(&mut Vec<RecentEntry>) -> bool) -> io::Result<()> {
        let mut stored = self.read();
        if change(&mut stored.entries) {
            self.write(&stored.entries, stored.backup_of_version)?;
        }
        Ok(())
    }

    /// Records an open, save or duplicate: `entry` goes first.
    pub fn record(&self, entry: RecentEntry) -> io::Result<()> {
        self.modify(|entries| {
            upsert(entries, entry);
            true
        })
    }

    /// The entries, newest first, with each file's modification time. Never removes an entry.
    /// Deletes cover files that no entry references.
    pub fn list(&self) -> Vec<Listed> {
        let stored = self.read();
        if stored.backup_of_version.is_none() {
            self.prune_covers(&stored.entries);
        }
        stored
            .entries
            .into_iter()
            .map(|entry| Listed {
                modified_ms: modified_ms(Path::new(&entry.path)),
                entry,
            })
            .collect()
    }

    /// Removes the entry for `path`; an unknown path is fine.
    pub fn remove(&self, path: &str) -> io::Result<()> {
        self.modify(|entries| remove(entries, path))
    }

    fn prune_covers(&self, entries: &[RecentEntry]) {
        let Ok(files) = fs::read_dir(&self.covers) else {
            return;
        };
        let used: HashSet<&str> = entries.iter().filter_map(|e| e.cover.as_deref()).collect();
        for file in files.flatten() {
            let name = file.file_name();
            let Some(hash) = name.to_str().and_then(|n| n.strip_suffix(".jpg")) else {
                continue;
            };
            if valid_hash(hash)
                && !used.contains(hash)
                && let Err(err) = fs::remove_file(file.path())
            {
                tracing::warn!(%err, hash, "cannot delete an unused cover");
            }
        }
    }

    /// The entry for `path`, if it is in the list and not the open project's file. Every file
    /// action checks this first, so the webview can only point them at projects the app recorded.
    fn guarded(&self, path: &str, open_file: Option<&Path>) -> Result<(Stored, usize), IpcError> {
        let stored = self.read();
        let index = stored
            .entries
            .iter()
            .position(|e| same_path(&e.path, path))
            .ok_or_else(|| IpcError::InvalidEdit(format!("{path} is not a recent project")))?;
        if open_file.is_some_and(|open| same_path(&open.to_string_lossy(), path)) {
            return Err(IpcError::InvalidEdit(format!("{path} is the open project")));
        }
        Ok((stored, index))
    }

    /// Renames the project file at `path` to `project_path_for(its folder, name)` and sets the
    /// project's name inside it. The entry keeps its `openedAtMs`.
    pub fn rename(&self, path: &str, name: &str, open_file: Option<&Path>) -> Result<(), IpcError> {
        let name = non_empty(name)?;
        let (mut stored, index) = self.guarded(path, open_file)?;
        let old = PathBuf::from(&stored.entries[index].path);
        let loaded = project_file::read_project_file(&old)?;
        let mut project = loaded.project;
        project.name = name.to_owned();

        let new = project_path_for(old.parent().unwrap_or(Path::new("")), name);
        let new_key = new.to_string_lossy().into_owned();
        // Differing only in case is the same file (Windows); anything else must be free.
        let same_file = same_path(&new_key, &stored.entries[index].path);
        if !same_file && new.exists() {
            return Err(IpcError::InvalidEdit(format!(
                "{} already exists",
                new.display()
            )));
        }
        let moved = old != new;
        if moved {
            fs::rename(&old, &new).map_err(|err| IpcError::io(&err))?;
        }
        let backup =
            (loaded.source_version < CURRENT_SCHEMA_VERSION).then_some(loaded.source_version);
        let json = serialization::to_json(&project);
        if let Err(err) = project_file::write_project_file(&new, &json, backup) {
            if moved && let Err(back) = fs::rename(&new, &old) {
                tracing::error!(%back, "cannot rename the project back after a failed write");
            }
            return Err(IpcError::io(&err));
        }

        let mut entry = stored.entries.remove(index);
        entry.path = new_key;
        entry.name = name.to_owned();
        // A stale (missing) entry may already have the new path.
        stored.entries.retain(|e| !same_path(&e.path, &entry.path));
        stored
            .entries
            .insert(index.min(stored.entries.len()), entry);
        self.write(&stored.entries, stored.backup_of_version)
            .map_err(|err| IpcError::io(&err))
    }

    /// Copies the project at `path` to `free_project_path(its folder, name)` with a new
    /// `ProjectId` and `name`, written in the current schema; the copy goes first in the list.
    pub fn duplicate(
        &self,
        path: &str,
        name: &str,
        open_file: Option<&Path>,
    ) -> Result<PathBuf, IpcError> {
        let name = non_empty(name)?;
        let (mut stored, index) = self.guarded(path, open_file)?;
        let source = PathBuf::from(&stored.entries[index].path);
        let mut project = project_file::read_project_file(&source)?.project;
        // Track, clip and media ids are project-scoped and stay.
        project.id = ProjectId::new();
        project.name = name.to_owned();

        let target =
            free_project_path(source.parent().unwrap_or(Path::new("")), name, Path::exists);
        project_file::write_project_file(&target, &serialization::to_json(&project), None)
            .map_err(|err| IpcError::io(&err))?;
        upsert(&mut stored.entries, RecentEntry::new(&project, &target));
        self.write(&stored.entries, stored.backup_of_version)
            .map_err(|err| IpcError::io(&err))?;
        Ok(target)
    }

    /// Moves the project file at `path` to the Recycle Bin with `trash` and removes its entry.
    /// `.v<n>.bak` siblings are left alone.
    pub fn trash(
        &self,
        path: &str,
        open_file: Option<&Path>,
        trash: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), IpcError> {
        let (mut stored, index) = self.guarded(path, open_file)?;
        let file = PathBuf::from(&stored.entries[index].path);
        if !file.is_file() {
            return Err(IpcError::FileNotFound(format!(
                "{} does not exist",
                file.display()
            )));
        }
        trash(&file).map_err(|err| IpcError::io(&err))?;
        stored.entries.remove(index);
        self.write(&stored.entries, stored.backup_of_version)
            .map_err(|err| IpcError::io(&err))
    }

    /// The file of the entry for `path`, checked like the other actions, and existing.
    pub fn existing_file(&self, path: &str, open_file: Option<&Path>) -> Result<PathBuf, IpcError> {
        let (stored, index) = self.guarded(path, open_file)?;
        let file = PathBuf::from(&stored.entries[index].path);
        if !file.exists() {
            return Err(IpcError::FileNotFound(format!(
                "{} does not exist",
                file.display()
            )));
        }
        Ok(file)
    }
}

fn broken() -> Stored {
    Stored {
        entries: Vec::new(),
        backup_of_version: Some(BROKEN_FILE_VERSION),
    }
}

fn non_empty(name: &str) -> Result<&str, IpcError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(IpcError::InvalidEdit("the project name is empty".into()));
    }
    Ok(name)
}

/// Modification time of the file at `path` in Unix milliseconds; `None` when it is not a
/// reachable file.
fn modified_ms(path: &Path) -> Option<i64> {
    let metadata = fs::metadata(path).ok().filter(fs::Metadata::is_file)?;
    let modified = metadata.modified().ok()?;
    let ms = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
    i64::try_from(ms).ok()
}

/// Moves `path` to the Recycle Bin (Windows: `IFileOperation` with `FOFX_RECYCLEONDELETE`). On a
/// drive without a Recycle Bin (e.g. a network share) Windows deletes the file permanently.
///
/// Runs on a fresh thread: `trash` initializes COM apartment-threaded on the calling thread and
/// panics if that thread already joined the multithreaded apartment, which a reused blocking-pool
/// thread may have.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("recycle-bin".into())
        .spawn(move || trash::delete(&path).map_err(|err| io::Error::other(err.to_string())))?
        .join()
        .map_err(|_| io::Error::other("moving to the Recycle Bin panicked"))?
}

/// Reads `covers/<hash>.jpg`; on a miss copies `poster` (the preview cache's poster for that hash)
/// there first. `None` when neither exists.
pub fn read_cover(
    covers: &Path,
    hash: &str,
    poster: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if !valid_hash(hash) {
        return None;
    }
    let path = covers.join(format!("{hash}.jpg"));
    if let Ok(bytes) = fs::read(&path) {
        return Some(bytes);
    }
    let bytes = poster()?;
    if let Err(err) = write_cover(&path, &bytes) {
        tracing::warn!(%err, hash, "cannot keep a cover");
    }
    Some(bytes)
}

/// Writes a cover atomically (unique temporary file, then rename), so a concurrent
/// reader never sees half a JPEG.
fn write_cover(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.tmp", std::process::id()));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use app_core::media::samples;

    use super::*;

    fn entry(path: &str, opened_at_ms: i64) -> RecentEntry {
        RecentEntry {
            path: path.into(),
            name: format!("Project {opened_at_ms}"),
            duration: Time::from_seconds(10),
            opened_at_ms,
            cover: None,
        }
    }

    fn paths(entries: &[RecentEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn upsert_dedupes_case_insensitively_and_moves_to_the_front() {
        let mut entries = vec![entry("C:\\a.cutframe", 1), entry("C:\\b.cutframe", 2)];
        upsert(&mut entries, entry("c:\\B.CUTFRAME", 3));
        assert_eq!(paths(&entries), ["c:\\B.CUTFRAME", "C:\\a.cutframe"]);
        assert_eq!(entries[0].opened_at_ms, 3);

        upsert(&mut entries, entry("C:\\Łódź.cutframe", 4));
        upsert(&mut entries, entry("C:\\łÓdŹ.cutframe", 5));
        assert_eq!(
            entries.len(),
            3,
            "non-ASCII names compare case-insensitively too"
        );
    }

    #[test]
    fn upsert_caps_the_list_dropping_the_oldest() {
        let mut entries = Vec::new();
        for n in 0..RECENT_CAP as i64 + 5 {
            upsert(&mut entries, entry(&format!("C:\\p{n}.cutframe"), n));
        }
        assert_eq!(entries.len(), RECENT_CAP);
        assert_eq!(entries[0].opened_at_ms, RECENT_CAP as i64 + 4);
        assert_eq!(entries.last().unwrap().opened_at_ms, 5);
    }

    #[test]
    fn remove_reports_whether_it_found_the_entry() {
        let mut entries = vec![entry("C:\\a.cutframe", 1), entry("C:\\b.cutframe", 2)];
        assert!(remove(&mut entries, "c:\\A.cutframe"));
        assert_eq!(paths(&entries), ["C:\\b.cutframe"]);
        assert!(!remove(&mut entries, "C:\\a.cutframe"));
    }

    #[test]
    fn project_paths_follow_the_file_name_rule() {
        let dir = Path::new("dir");
        let name = |n: &str| {
            project_path_for(dir, n)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(name("Holiday"), "Holiday.cutframe");
        assert_eq!(
            name("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j.cutframe"
        );
        assert_eq!(name("tab\there\u{1}"), "tab_here_.cutframe");
        assert_eq!(name("Trip. "), "Trip.cutframe");
        assert_eq!(name("  Trip...  "), "Trip.cutframe");
        assert_eq!(name(" . . "), "Untitled.cutframe");
        assert_eq!(name(""), "Untitled.cutframe");
        assert_eq!(project_path_for(dir, "x"), dir.join("x.cutframe"));
    }

    #[test]
    fn free_project_paths_count_up() {
        let dir = Path::new("dir");
        let taken = RefCell::new(HashSet::new());
        let free = || {
            let path = free_project_path(dir, "Trip.", |p| taken.borrow().contains(p));
            taken.borrow_mut().insert(path.clone());
            path.file_name().unwrap().to_string_lossy().into_owned()
        };
        assert_eq!(free(), "Trip.cutframe");
        assert_eq!(free(), "Trip (2).cutframe");
        assert_eq!(free(), "Trip (3).cutframe");
    }

    fn fixture_path(version: u32) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("recent-v{version}.json"))
    }

    #[test]
    fn every_fixture_loads() {
        for version in 1..=CURRENT_RECENT_VERSION {
            let path = fixture_path(version);
            let text =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let (entries, source) = load(&text).unwrap_or_else(|err| panic!("v{version}: {err}"));
            assert_eq!(source, version);
            assert!(!entries.is_empty(), "v{version} fixture has entries");
        }
    }

    #[test]
    fn the_v1_fixture_reads_as_written() {
        let (entries, _) = load(&fs::read_to_string(fixture_path(1)).unwrap()).unwrap();
        assert_eq!(
            entries,
            [
                RecentEntry {
                    path: "C:\\Users\\me\\Videos\\Holiday.cutframe".into(),
                    name: "Holiday".into(),
                    duration: Time::from_seconds(95),
                    opened_at_ms: 1_795_000_000_000,
                    cover: Some("0123456789abcdef0123456789abcdef".into()),
                },
                RecentEntry {
                    path: "D:\\Projects\\Short.cutframe".into(),
                    name: "Short".into(),
                    duration: Time::ZERO,
                    opened_at_ms: 1_794_000_000_000,
                    cover: None,
                },
            ]
        );
    }

    #[test]
    fn load_rejects_files_without_a_version_and_newer_files() {
        assert!(matches!(load("not json"), Err(LoadError::Json(_))));
        assert!(matches!(load("{}"), Err(LoadError::MissingVersion)));
        assert!(matches!(
            load(r#"{"schemaVersion": 0, "entries": []}"#),
            Err(LoadError::MissingVersion)
        ));
        assert!(matches!(
            load(r#"{"schemaVersion": 2, "entries": []}"#),
            Err(LoadError::TooNew { found: 2 })
        ));
        assert!(matches!(
            load(r#"{"schemaVersion": 1, "entries": [{"path": 1}]}"#),
            Err(LoadError::Json(_))
        ));
    }

    struct Dir {
        dir: tempfile::TempDir,
        store: RecentStore,
    }

    fn store() -> Dir {
        let dir = tempfile::tempdir().unwrap();
        let store = RecentStore::new(&dir.path().join("app-data"));
        Dir { dir, store }
    }

    impl Dir {
        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        /// Writes `project` to `name` and records it, like a save.
        fn saved(&self, name: &str, project: &Project) -> String {
            let path = self.path(name);
            project_file::write_project_file(&path, &serialization::to_json(project), None)
                .unwrap();
            self.store.record(RecentEntry::new(project, &path)).unwrap();
            path.to_string_lossy().into_owned()
        }

        fn entries(&self) -> Vec<RecentEntry> {
            self.store.list().into_iter().map(|l| l.entry).collect()
        }

        fn app_data_files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.path("app-data"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    fn kind(result: Result<impl std::fmt::Debug, IpcError>) -> String {
        serde_json::to_value(result.unwrap_err()).unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn a_missing_file_is_an_empty_list_and_recording_creates_it() {
        let d = store();
        assert!(d.store.list().is_empty());
        let project = Project::new("Saved");
        let path = d.saved("a.cutframe", &project);
        let listed = d.store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].entry.path, path);
        assert_eq!(listed[0].entry.name, "Saved");
        assert!(listed[0].modified_ms.is_some());
        assert_eq!(d.app_data_files(), ["recent.json"]);
    }

    #[test]
    fn broken_and_newer_files_list_empty_and_are_backed_up_on_the_next_write() {
        for (text, backup) in [
            ("not json at all", "recent.json.v0.bak"),
            (
                r#"{"schemaVersion": 7, "entries": []}"#,
                "recent.json.v7.bak",
            ),
        ] {
            let d = store();
            fs::create_dir_all(d.path("app-data")).unwrap();
            fs::write(d.path("app-data").join(FILE_NAME), text).unwrap();
            assert!(d.store.list().is_empty());
            assert_eq!(
                d.app_data_files(),
                ["recent.json"],
                "listing writes nothing"
            );

            d.saved("a.cutframe", &Project::new("A"));
            assert_eq!(
                d.app_data_files(),
                [FILE_NAME.to_owned(), backup.to_owned()]
            );
            let kept = fs::read_to_string(d.path("app-data").join(backup)).unwrap();
            assert_eq!(kept, text);
            assert_eq!(d.entries().len(), 1);

            // The file is valid now: no further backups.
            d.saved("b.cutframe", &Project::new("B"));
            assert_eq!(d.app_data_files().len(), 2);
        }
    }

    #[test]
    fn listing_reports_missing_files_and_keeps_their_entries() {
        let d = store();
        let path = d.saved("a.cutframe", &Project::new("A"));
        fs::remove_file(&path).unwrap();
        let listed = d.store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].modified_ms, None);
        assert_eq!(d.store.list().len(), 1, "never pruned");

        d.store.remove(&path.to_uppercase()).unwrap();
        assert!(d.store.list().is_empty());
        d.store.remove(&path).unwrap();
    }

    #[test]
    fn rename_moves_the_file_and_renames_the_project() {
        let d = store();
        let path = d.saved("Old.cutframe", &Project::new("Old"));
        let opened_at = d.entries()[0].opened_at_ms;

        d.store.rename(&path, "  New: cut ", None).unwrap();
        let new = d.path("New_ cut.cutframe");
        assert!(!Path::new(&path).exists());
        let loaded = project_file::read_project_file(&new).unwrap();
        assert_eq!(loaded.project.name, "New: cut");
        let entries = d.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, new.to_string_lossy());
        assert_eq!(entries[0].name, "New: cut");
        assert_eq!(entries[0].opened_at_ms, opened_at);
        assert_eq!(
            fs::read_dir(d.dir.path()).unwrap().count(),
            2,
            "the project and app-data, no leftovers"
        );
    }

    #[test]
    fn rename_to_the_same_name_in_another_case_keeps_the_file() {
        let d = store();
        let path = d.saved("Trip.cutframe", &Project::new("Trip"));
        d.store.rename(&path, "Trip", None).unwrap();
        assert_eq!(
            project_file::read_project_file(Path::new(&path))
                .unwrap()
                .project
                .name,
            "Trip"
        );
        d.store.rename(&path, "TRIP", None).unwrap();
        let entries = d.entries();
        assert_eq!(entries[0].name, "TRIP");
        let renamed = PathBuf::from(&entries[0].path);
        assert_eq!(
            project_file::read_project_file(&renamed)
                .unwrap()
                .project
                .name,
            "TRIP"
        );
    }

    #[test]
    fn rename_refuses_an_existing_target_and_an_empty_name() {
        let d = store();
        let path = d.saved("A.cutframe", &Project::new("A"));
        d.saved("B.cutframe", &Project::new("B"));
        assert_eq!(kind(d.store.rename(&path, "B", None)), "invalidEdit");
        assert_eq!(kind(d.store.rename(&path, "   ", None)), "invalidEdit");
        assert_eq!(
            project_file::read_project_file(Path::new(&path))
                .unwrap()
                .project
                .name,
            "A",
            "unchanged"
        );
    }

    #[test]
    fn rename_backs_up_an_older_schema_file() {
        let d = store();
        let path = d.path("Old.cutframe");
        let old = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/app-core/tests/fixtures/project-v1.json");
        fs::copy(&old, &path).unwrap();
        let loaded = project_file::read_project_file(&path).unwrap();
        assert!(
            loaded.source_version < CURRENT_SCHEMA_VERSION,
            "fixture is old"
        );
        d.store
            .record(RecentEntry::new(&loaded.project, &path))
            .unwrap();

        d.store
            .rename(&path.to_string_lossy(), "Renamed", None)
            .unwrap();
        let backup = d.path(&format!("Renamed.cutframe.v{}.bak", loaded.source_version));
        assert_eq!(fs::read(&backup).unwrap(), fs::read(&old).unwrap());
        let renamed = project_file::read_project_file(&d.path("Renamed.cutframe")).unwrap();
        assert_eq!(renamed.source_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(renamed.project.name, "Renamed");
    }

    #[test]
    fn file_actions_refuse_paths_not_in_the_list_and_the_open_file() {
        let d = store();
        let listed = d.saved("A.cutframe", &Project::new("A"));
        let stranger = d.path("Stranger.cutframe");
        project_file::write_project_file(
            &stranger,
            &serialization::to_json(&Project::new("S")),
            None,
        )
        .unwrap();
        let stranger = stranger.to_string_lossy().into_owned();
        let open = PathBuf::from(listed.to_uppercase());
        let never = |_: &Path| -> io::Result<()> { panic!("must not trash") };

        for (path, open_file) in [(&stranger, None), (&listed, Some(open.as_path()))] {
            assert_eq!(kind(d.store.rename(path, "X", open_file)), "invalidEdit");
            assert_eq!(kind(d.store.duplicate(path, "X", open_file)), "invalidEdit");
            assert_eq!(kind(d.store.trash(path, open_file, never)), "invalidEdit");
            assert_eq!(kind(d.store.existing_file(path, open_file)), "invalidEdit");
        }
        assert!(Path::new(&stranger).exists() && Path::new(&listed).exists());
        assert_eq!(d.entries().len(), 1);
    }

    #[test]
    fn duplicate_writes_a_new_project_first_in_the_list() {
        let d = store();
        let mut project = Project::new("Trip");
        project.media = vec![samples::video(1)];
        let path = d.saved("Trip.cutframe", &project);

        let copy = d.store.duplicate(&path, "Trip copy", None).unwrap();
        assert_eq!(copy, d.path("Trip copy.cutframe"));
        let loaded = project_file::read_project_file(&copy).unwrap().project;
        assert_ne!(loaded.id, project.id);
        assert_eq!(loaded.name, "Trip copy");
        assert_eq!(loaded.media, project.media, "other ids are kept");

        let again = d.store.duplicate(&path, "Trip copy", None).unwrap();
        assert_eq!(again, d.path("Trip copy (2).cutframe"));
        let entries = d.entries();
        assert_eq!(
            paths(&entries),
            [
                again.to_string_lossy().as_ref(),
                copy.to_string_lossy().as_ref(),
                path.as_str()
            ]
        );
        assert_eq!(entries[0].cover.as_deref(), app_core::cover_hash(&project));
        assert_eq!(kind(d.store.duplicate(&path, " ", None)), "invalidEdit");
    }

    #[test]
    fn duplicate_of_a_missing_file_is_file_not_found() {
        let d = store();
        let path = d.saved("A.cutframe", &Project::new("A"));
        fs::remove_file(&path).unwrap();
        assert_eq!(kind(d.store.duplicate(&path, "B", None)), "fileNotFound");
    }

    #[test]
    fn trash_moves_the_file_through_the_seam_and_removes_the_entry() {
        let d = store();
        let path = d.saved("A.cutframe", &Project::new("A"));
        let keep = d.saved("B.cutframe", &Project::new("B"));
        let trashed = RefCell::new(Vec::new());
        d.store
            .trash(&path, None, |p| {
                trashed.borrow_mut().push(p.to_path_buf());
                fs::remove_file(p)
            })
            .unwrap();
        assert_eq!(*trashed.borrow(), [PathBuf::from(&path)]);
        assert_eq!(paths(&d.entries()), [keep.as_str()]);

        // A failing Recycle Bin keeps the entry.
        let failed = d
            .store
            .trash(&keep, None, |_| Err(io::Error::other("no recycle bin")));
        assert_eq!(kind(failed), "io");
        assert_eq!(d.entries().len(), 1);

        fs::remove_file(&keep).unwrap();
        assert_eq!(kind(d.store.trash(&keep, None, |_| Ok(()))), "fileNotFound");
    }

    #[test]
    fn listing_deletes_unused_covers() {
        let d = store();
        let mut project = Project::new("A");
        project.media = vec![samples::video(1)];
        let used = app_core::cover_hash(&project).unwrap().to_owned();
        d.saved("A.cutframe", &project);
        let covers = d.store.covers_dir().to_path_buf();
        fs::create_dir_all(&covers).unwrap();
        let unused = "ffffffffffffffffffffffffffffffff";
        for name in [
            format!("{used}.jpg"),
            format!("{unused}.jpg"),
            "notes.txt".into(),
        ] {
            fs::write(covers.join(name), b"x").unwrap();
        }
        d.store.list();
        let mut left: Vec<String> = fs::read_dir(&covers)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, [format!("{used}.jpg"), "notes.txt".into()]);
    }

    #[test]
    fn covers_are_kept_from_the_preview_cache() {
        let d = store();
        let covers = d.store.covers_dir();
        let hash = "0123456789abcdef0123456789abcdef";
        assert_eq!(read_cover(covers, hash, || None), None, "miss");
        assert_eq!(
            read_cover(covers, hash, || Some(b"jpeg".to_vec())),
            Some(b"jpeg".to_vec())
        );
        assert_eq!(
            fs::read(covers.join(format!("{hash}.jpg"))).unwrap(),
            b"jpeg"
        );
        // Served from the covers folder now, even when the cache was cleared.
        assert_eq!(
            read_cover(covers, hash, || panic!("not asked")),
            Some(b"jpeg".to_vec())
        );
        assert_eq!(
            fs::read_dir(covers).unwrap().count(),
            1,
            "no temporary files"
        );
        assert_eq!(
            read_cover(covers, "../../etc", || panic!("not asked")),
            None
        );
    }
}
