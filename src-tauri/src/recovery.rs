//! Recovery store: autosave envelopes in the app-data `recovery/` directory (design
//! `autosave-recovery.md` §4).
//!
//! Plain blocking functions, no Tauri: callers run them off the main thread (the autosave thread
//! or `spawn_blocking`). Every process has one session id and owns `<id>.json` (the envelope) and
//! `<id>.lock`, which it keeps locked for its lifetime. An entry whose lock file is missing or can
//! be locked belongs to a process that is gone: it is *orphaned* and may be offered or pruned.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use app_core::Project;
use app_core::serialization::{self, CURRENT_SCHEMA_VERSION, LoadError};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::project_file::{self, ReadError};
use crate::state::SaveSnapshot;

/// Format of [`RecoveryFile`]. An envelope with another value is skipped and kept, so a newer app
/// can still read it.
pub const RECOVERY_VERSION: u32 = 1;

const JSON_SUFFIX: &str = ".json";
const LOCK_SUFFIX: &str = ".lock";
/// Left behind by a write that was interrupted by a crash (`project_file::write_project_file`).
const TMP_SUFFIX: &str = ".json.tmp";

/// One autosave: the project and the session metadata it needs to be restored. One file, so
/// content and metadata are replaced together by one rename.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryFile {
    recovery_version: u32,
    session_id: Uuid,
    /// Unix ms when the snapshot was taken.
    saved_at_ms: i64,
    /// The session's project file at that time; `None` = never saved.
    file_path: Option<PathBuf>,
    /// The session's `source_version`, so a restored session keeps the `.v<N>.bak` rule.
    source_version: u32,
    /// Exactly `serialization::to_json(&project)`.
    project: Box<RawValue>,
}

/// Only the version, read first so an envelope of an unknown shape is not mistaken for corrupt.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryHeader {
    recovery_version: u32,
}

/// An orphaned recovery entry that can be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Session id of the process that wrote it.
    pub id: Uuid,
    pub project_name: String,
    pub file_path: Option<PathBuf>,
    pub saved_at_ms: i64,
}

/// A loaded entry, ready to become the open project.
#[derive(Debug)]
pub struct Restored {
    pub project: Project,
    pub file_path: Option<PathBuf>,
    pub source_version: u32,
}

/// The recovery directory and this process's session id.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    dir: PathBuf,
    session_id: Uuid,
}

impl RecoveryStore {
    pub fn new(dir: PathBuf, session_id: Uuid) -> Self {
        Self { dir, session_id }
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Creates the directory and this session's lock file and locks it. Keep the returned handle
    /// open for the whole process lifetime: the OS releases the lock when the process dies, also
    /// on a crash, which is what marks the entry as orphaned.
    pub fn acquire_lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path(self.session_id))?;
        file.try_lock().map_err(|err| match err {
            TryLockError::Error(err) => err,
            TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "own recovery lock is held by another handle",
            ),
        })?;
        Ok(file)
    }

    /// Writes `snapshot` as this session's envelope, atomically (tmp + rename). Never touches
    /// `snapshot.file_path`.
    pub fn write(&self, snapshot: &SaveSnapshot, saved_at_ms: i64) -> io::Result<()> {
        let project = RawValue::from_string(serialization::to_json(&snapshot.project))
            .map_err(io::Error::other)?;
        let envelope = RecoveryFile {
            recovery_version: RECOVERY_VERSION,
            session_id: self.session_id,
            saved_at_ms,
            file_path: snapshot.file_path.clone(),
            source_version: snapshot.source_version,
            project,
        };
        // Fails only for a path that is not valid Unicode.
        let json = serde_json::to_string(&envelope)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        project_file::write_project_file(&self.json_path(self.session_id), &json, None)
    }

    /// Deletes this session's envelope. A missing file is fine.
    pub fn delete_own(&self) -> io::Result<()> {
        remove_if_exists(&self.json_path(self.session_id))
    }

    /// Deletes this session's envelope and lock file (clean exit). Drop the lock handle first:
    /// Windows does not delete a file that is still open without `FILE_SHARE_DELETE`.
    pub fn delete_own_all(&self) -> io::Result<()> {
        self.delete_files(self.session_id)
    }

    /// Orphaned entries that are newer than their project file, newest first. Prunes corrupt and
    /// stale entries and leftover lock and temporary files on the way. Entries of running
    /// instances and of this session are never touched. A missing directory gives an empty list;
    /// a problem with one entry never stops the scan.
    pub fn list_orphans(&self) -> io::Result<Vec<Entry>> {
        let read = match fs::read_dir(&self.dir) {
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut sessions: BTreeMap<Uuid, Files> = BTreeMap::new();
        for item in read {
            let Ok(item) = item else { continue };
            let name = item.file_name();
            let Some((id, kind)) = name.to_str().and_then(parse_name) else {
                continue;
            };
            let files = sessions.entry(id).or_default();
            match kind {
                FileKind::Json => files.json = true,
                FileKind::Lock => files.lock = true,
                FileKind::Tmp => files.tmp = true,
            }
        }

        let mut entries = Vec::new();
        for (id, files) in sessions {
            if id == self.session_id || !self.is_orphaned(id) {
                continue;
            }
            if files.tmp {
                warn_on_err(remove_if_exists(&self.tmp_path(id)), id);
            }
            if !files.json {
                tracing::info!(%id, "removing a recovery lock file without an autosave");
                warn_on_err(remove_if_exists(&self.lock_path(id)), id);
                continue;
            }
            match self.read_entry(id) {
                Ok((entry, _)) => {
                    let file_mtime_ms = entry.file_path.as_deref().and_then(mtime_ms);
                    if should_offer(entry.saved_at_ms, entry.file_path.as_deref(), file_mtime_ms) {
                        entries.push(entry);
                    } else {
                        tracing::info!(%id, "removing an autosave older than its project file");
                        warn_on_err(self.delete_files(id), id);
                    }
                }
                Err(err) if keeps_entry(&err) => {
                    tracing::warn!(%id, %err, "keeping a recovery entry this app cannot read");
                }
                Err(err) => {
                    tracing::warn!(%id, %err, "removing a corrupt recovery entry");
                    warn_on_err(self.delete_files(id), id);
                }
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.saved_at_ms));
        Ok(entries)
    }

    /// Loads the orphaned entry `id` (full open pipeline: migrate, validate). An entry that does
    /// not exist, belongs to this session or to a running instance is `NotFound`.
    pub fn load(&self, id: Uuid) -> Result<Restored, ReadError> {
        if id == self.session_id || !self.is_orphaned(id) {
            return Err(not_found());
        }
        self.read_entry(id).map(|(_, restored)| restored)
    }

    /// Deletes the orphaned entry `id`. Unknown ids, this session and running instances are a
    /// no-op.
    pub fn discard(&self, id: Uuid) -> io::Result<()> {
        if id == self.session_id || !self.is_orphaned(id) {
            return Ok(());
        }
        self.delete_files(id)
    }

    /// Deletes another session's entry after it was restored into this one. Unlike [`discard`],
    /// it does not check the lock: the caller loaded it as an orphan.
    ///
    /// [`discard`]: RecoveryStore::discard
    pub fn delete_adopted(&self, id: Uuid) -> io::Result<()> {
        if id == self.session_id {
            return Ok(());
        }
        self.delete_files(id)
    }

    fn read_entry(&self, id: Uuid) -> Result<(Entry, Restored), ReadError> {
        let text = fs::read_to_string(self.json_path(id))?;
        let header: RecoveryHeader = serde_json::from_str(&text).map_err(LoadError::Json)?;
        if header.recovery_version != RECOVERY_VERSION {
            return Err(ReadError::Load(LoadError::TooNew {
                found: header.recovery_version.into(),
                supported: RECOVERY_VERSION,
            }));
        }
        let envelope: RecoveryFile = serde_json::from_str(&text).map_err(LoadError::Json)?;
        let loaded = serialization::load(envelope.project.get())?;
        let entry = Entry {
            id,
            project_name: loaded.project.name.clone(),
            file_path: envelope.file_path.clone(),
            saved_at_ms: envelope.saved_at_ms,
        };
        let restored = Restored {
            project: loaded.project,
            file_path: envelope.file_path,
            // `to_json` always writes the current version; the session's own source version is
            // what decides the backup on its next save.
            source_version: envelope.source_version.min(CURRENT_SCHEMA_VERSION),
        };
        Ok((entry, restored))
    }

    /// The lock file is missing, or it can be locked (its owner is gone). The probe handle is
    /// dropped right away, which releases the probe's lock.
    fn is_orphaned(&self, id: Uuid) -> bool {
        let file = match File::open(self.lock_path(id)) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return true,
            // Cannot tell; leave it alone.
            Err(_) => return false,
        };
        match file.try_lock() {
            Ok(()) => true,
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Error(_)) => false,
        }
    }

    /// Envelope first, then the lock file, so a half-done delete still looks orphaned.
    fn delete_files(&self, id: Uuid) -> io::Result<()> {
        remove_if_exists(&self.json_path(id))?;
        remove_if_exists(&self.tmp_path(id))?;
        remove_if_exists(&self.lock_path(id))
    }

    fn json_path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}{JSON_SUFFIX}"))
    }

    fn lock_path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}{LOCK_SUFFIX}"))
    }

    fn tmp_path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}{TMP_SUFFIX}"))
    }
}

/// Whether an autosave taken at `saved_at_ms` should be offered: always when the project was never
/// saved or its file is gone, otherwise only when the autosave is newer than the file.
pub fn should_offer(
    saved_at_ms: i64,
    file_path: Option<&Path>,
    file_mtime_ms: Option<i64>,
) -> bool {
    match (file_path, file_mtime_ms) {
        (None, _) | (Some(_), None) => true,
        (Some(_), Some(mtime)) => saved_at_ms > mtime,
    }
}

/// Milliseconds since the Unix epoch.
pub fn unix_ms(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(err) => -i64::try_from(err.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

fn mtime_ms(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(unix_ms)
}

/// Errors that mean "a newer app wrote this": keep the entry.
fn keeps_entry(err: &ReadError) -> bool {
    match err {
        ReadError::Load(LoadError::TooNew { .. }) => true,
        // A file that vanished or cannot be read right now (e.g. locked by a scanner) is not
        // known to be corrupt. Not UTF-8 is.
        ReadError::Io(e) => e.kind() != io::ErrorKind::InvalidData,
        ReadError::Load(
            LoadError::Json(_)
            | LoadError::MissingVersion
            | LoadError::Invalid(_)
            | LoadError::Migration { .. },
        ) => false,
    }
}

fn not_found() -> ReadError {
    ReadError::Io(io::Error::new(
        io::ErrorKind::NotFound,
        "no orphaned recovery entry with this id",
    ))
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn warn_on_err(result: io::Result<()>, id: Uuid) {
    if let Err(err) = result {
        tracing::warn!(%id, %err, "failed to delete a recovery file");
    }
}

#[derive(Debug, Default)]
struct Files {
    json: bool,
    lock: bool,
    tmp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Json,
    Lock,
    Tmp,
}

/// `<uuid>.json`, `<uuid>.lock` or `<uuid>.json.tmp`; anything else is not ours.
fn parse_name(name: &str) -> Option<(Uuid, FileKind)> {
    let (stem, kind) = if let Some(stem) = name.strip_suffix(TMP_SUFFIX) {
        (stem, FileKind::Tmp)
    } else if let Some(stem) = name.strip_suffix(JSON_SUFFIX) {
        (stem, FileKind::Json)
    } else {
        (name.strip_suffix(LOCK_SUFFIX)?, FileKind::Lock)
    };
    Some((Uuid::try_parse(stem).ok()?, kind))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use super::*;
    use crate::state::AppState;

    pub(crate) fn store(dir: &Path) -> RecoveryStore {
        RecoveryStore::new(dir.to_path_buf(), Uuid::now_v7())
    }

    fn snapshot(name: &str, file_path: Option<PathBuf>) -> SaveSnapshot {
        let state = AppState::new(
            Project::new(name),
            None,
            store(Path::new("unused")),
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        );
        if let Some(path) = file_path {
            state.test_replace(Project::new(name), Some(path), CURRENT_SCHEMA_VERSION);
        }
        state.save_snapshot()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A crashed session: an envelope and an unlocked lock file.
    fn crashed(dir: &Path, name: &str, saved_at_ms: i64, file_path: Option<PathBuf>) -> Uuid {
        let other = store(dir);
        drop(other.acquire_lock().unwrap());
        other
            .write(&snapshot(name, file_path), saved_at_ms)
            .unwrap();
        other.session_id()
    }

    #[test]
    fn offers_only_autosaves_newer_than_the_file() {
        let path = Path::new("/p/a.cutframe");
        assert!(should_offer(10, None, None), "never saved");
        assert!(should_offer(10, None, Some(20)), "never saved");
        assert!(should_offer(10, Some(path), None), "file missing");
        assert!(should_offer(21, Some(path), Some(20)), "newer");
        assert!(!should_offer(20, Some(path), Some(20)), "equal");
        assert!(!should_offer(19, Some(path), Some(20)), "older");
    }

    #[test]
    fn envelope_round_trips_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut snap = snapshot("Round trip", Some(PathBuf::from("/p/missing.cutframe")));
        snap.source_version = 0;
        s.write(&snap, 1_234).unwrap();

        let text = fs::read_to_string(s.json_path(s.session_id())).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["recoveryVersion"], 1);
        assert_eq!(value["savedAtMs"], 1_234);
        assert_eq!(value["sourceVersion"], 0);
        assert_eq!(value["sessionId"], s.session_id().to_string());
        assert_eq!(
            value["project"],
            serde_json::from_str::<serde_json::Value>(&serialization::to_json(&snap.project))
                .unwrap()
        );

        let other = store(dir.path());
        let restored = other.load(s.session_id()).unwrap();
        assert_eq!(restored.project, *snap.project);
        assert_eq!(
            restored.file_path.as_deref(),
            Some(Path::new("/p/missing.cutframe"))
        );
        assert_eq!(restored.source_version, 0);
        let listed = other.list_orphans().unwrap();
        assert_eq!(
            listed,
            [Entry {
                id: s.session_id(),
                project_name: "Round trip".into(),
                file_path: Some(PathBuf::from("/p/missing.cutframe")),
                saved_at_ms: 1_234,
            }]
        );
    }

    #[test]
    fn a_write_never_touches_the_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let recovery = dir.path().join("recovery");
        let project = dir.path().join("mine.cutframe");
        fs::write(&project, "user's file").unwrap();
        let s = store(&recovery);
        let _lock = s.acquire_lock().unwrap();
        s.write(&snapshot("P", Some(project.clone())), 1).unwrap();

        assert_eq!(fs::read_to_string(&project).unwrap(), "user's file");
        assert_eq!(names(dir.path()), ["mine.cutframe", "recovery"]);
        let id = s.session_id();
        assert_eq!(
            names(&recovery),
            [format!("{id}.json"), format!("{id}.lock")]
        );
    }

    #[test]
    fn lists_newest_first_and_excludes_the_own_session() {
        let dir = tempfile::tempdir().unwrap();
        let older = crashed(dir.path(), "Older", 100, None);
        let newer = crashed(dir.path(), "Newer", 200, None);
        let own = store(dir.path());
        let _lock = own.acquire_lock().unwrap();
        own.write(&snapshot("Own", None), 300).unwrap();

        let ids: Vec<Uuid> = own.list_orphans().unwrap().iter().map(|e| e.id).collect();
        assert_eq!(ids, [newer, older]);
        assert!(matches!(
            own.load(own.session_id()),
            Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::NotFound
        ));
    }

    #[test]
    fn skips_and_never_touches_a_live_instance() {
        let dir = tempfile::tempdir().unwrap();
        let live = store(dir.path());
        let held = live.acquire_lock().unwrap();
        live.write(&snapshot("Live", None), 1).unwrap();
        let before = names(dir.path());

        let me = store(dir.path());
        assert_eq!(me.list_orphans().unwrap(), []);
        assert!(matches!(
            me.load(live.session_id()),
            Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::NotFound
        ));
        me.discard(live.session_id()).unwrap();
        assert_eq!(names(dir.path()), before);

        drop(held);
        assert_eq!(
            me.list_orphans().unwrap().len(),
            1,
            "orphaned once released"
        );
    }

    #[test]
    fn an_entry_without_a_lock_file_is_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let id = crashed(dir.path(), "No lock", 1, None);
        fs::remove_file(dir.path().join(format!("{id}.lock"))).unwrap();
        assert_eq!(store(dir.path()).list_orphans().unwrap().len(), 1);
    }

    #[test]
    fn prunes_corrupt_invalid_and_stale_entries_and_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());

        let corrupt = crashed(dir.path(), "Corrupt", 1, None);
        fs::write(s.json_path(corrupt), "not json").unwrap();

        let invalid = crashed(dir.path(), "Invalid", 1, None);
        let text = fs::read_to_string(s.json_path(invalid)).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["project"]["name"] = "".into();
        fs::write(s.json_path(invalid), value.to_string()).unwrap();

        let saved_file = dir.path().join("saved.cutframe");
        fs::write(&saved_file, "saved later").unwrap();
        let file_ms = mtime_ms(&saved_file).unwrap();
        let _stale = crashed(dir.path(), "Stale", file_ms - 1, Some(saved_file.clone()));
        let fresh = crashed(dir.path(), "Fresh", file_ms + 1, Some(saved_file));

        let lock_only = Uuid::now_v7();
        File::create(s.lock_path(lock_only)).unwrap();
        let tmp_only = Uuid::now_v7();
        fs::write(s.tmp_path(tmp_only), "half written").unwrap();
        fs::write(dir.path().join("unrelated.txt"), "not ours").unwrap();

        let listed = s.list_orphans().unwrap();
        assert_eq!(listed.iter().map(|e| e.id).collect::<Vec<_>>(), [fresh]);
        assert_eq!(
            names(dir.path()),
            [
                format!("{fresh}.json"),
                format!("{fresh}.lock"),
                "saved.cutframe".to_owned(),
                "unrelated.txt".to_owned(),
            ]
        );
    }

    #[test]
    fn keeps_entries_written_by_a_newer_app() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());

        let newer_envelope = crashed(dir.path(), "Envelope", 1, None);
        fs::write(
            s.json_path(newer_envelope),
            r#"{"recoveryVersion": 2, "somethingElse": true}"#,
        )
        .unwrap();

        let newer_project = crashed(dir.path(), "Project", 1, None);
        let text = fs::read_to_string(s.json_path(newer_project)).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["project"]["schemaVersion"] = 999.into();
        fs::write(s.json_path(newer_project), value.to_string()).unwrap();
        let before = names(dir.path());

        assert_eq!(s.list_orphans().unwrap(), []);
        assert_eq!(names(dir.path()), before);
        assert!(matches!(
            s.load(newer_envelope),
            Err(ReadError::Load(LoadError::TooNew { .. }))
        ));
        assert!(matches!(
            s.load(newer_project),
            Err(ReadError::Load(LoadError::TooNew { .. }))
        ));
    }

    #[test]
    fn a_missing_directory_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir.path().join("missing"));
        assert_eq!(s.list_orphans().unwrap(), []);
        s.discard(Uuid::now_v7()).unwrap();
    }

    #[test]
    fn discard_deletes_an_orphan_and_ignores_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        let id = crashed(dir.path(), "Gone", 1, None);
        let s = store(dir.path());
        s.discard(id).unwrap();
        s.discard(id).unwrap();
        s.discard(Uuid::now_v7()).unwrap();
        assert_eq!(names(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn unix_ms_counts_from_the_epoch() {
        assert_eq!(unix_ms(UNIX_EPOCH + Duration::from_millis(1_500)), 1_500);
        assert_eq!(unix_ms(UNIX_EPOCH - Duration::from_millis(5)), -5);
    }
}
