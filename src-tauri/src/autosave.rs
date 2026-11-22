//! Autosave thread (design `autosave-recovery.md` §5): while the project has unsaved changes,
//! writes it to the recovery directory at the interval set in the preferences (or never, when
//! autosave is off), never over the user's file.
//!
//! The thread owns every write and delete of this session's recovery files, so they are serialized
//! without a lock and the main thread does no I/O. The decisions live in [`Autosaver`]; the loop
//! only feeds it the clock and the trigger, so tests drive it directly.

use std::fs::File;
use std::io;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use app_core::HistoryPosition;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use uuid::Uuid;

use crate::recovery::{RecoveryStore, unix_ms};
use crate::state::SaveSnapshot;

/// How long a clean exit waits for the thread to delete its files, so a stuck disk cannot hang it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum Msg {
    /// The open project was saved explicitly or replaced (new/open): delete the own autosave.
    Discard,
    /// Entry `old` was restored into this session: write the own autosave now, then delete `old`.
    Adopt { old: Uuid },
    /// The autosave interval changed (`None` = off). The next tick is one new interval from now.
    /// Off also deletes the own autosave, so no stale snapshot is offered later.
    SetInterval(Option<Duration>),
    /// Clean exit: delete the own autosave and lock file, acknowledge, stop.
    Shutdown { ack: Sender<()> },
}

/// Sends messages to the autosave thread without blocking. Lives in `AppState`.
#[derive(Debug, Clone)]
pub struct AutosaveHandle {
    tx: Sender<Msg>,
}

impl AutosaveHandle {
    pub fn send(&self, msg: Msg) {
        // The thread only stops after `Shutdown`; nothing is left to do then.
        let _ = self.tx.send(msg);
    }

    /// Asks the thread to clean up and waits for it, at most [`SHUTDOWN_TIMEOUT`].
    pub fn shutdown(&self) {
        let (ack, done) = crossbeam_channel::bounded(1);
        self.send(Msg::Shutdown { ack });
        if done.recv_timeout(SHUTDOWN_TIMEOUT).is_err() {
            tracing::warn!("autosave thread did not finish cleaning up in time");
        }
    }
}

/// Starts the `autosave` thread. `lock` is this session's held recovery lock (or `None` if it
/// could not be taken); the thread keeps it open until `Shutdown`. `interval` is the autosave
/// interval from the preferences (`None` = off). `snapshot` takes the state to save (a brief
/// document lock).
pub fn spawn(
    store: RecoveryStore,
    lock: Option<File>,
    interval: Option<Duration>,
    snapshot: impl Fn() -> SaveSnapshot + Send + 'static,
) -> io::Result<AutosaveHandle> {
    let (tx, rx) = crossbeam_channel::unbounded();
    thread::Builder::new()
        .name("autosave".into())
        .spawn(move || run(&rx, &store, Autosaver::new(lock), interval, &snapshot))?;
    Ok(AutosaveHandle { tx })
}

/// When the next tick is due: one `interval` after `now`, or never when autosave is off.
pub fn next_deadline(now: Instant, interval: Option<Duration>) -> Option<Instant> {
    interval.map(|interval| now + interval)
}

fn run(
    rx: &Receiver<Msg>,
    store: &RecoveryStore,
    mut saver: Autosaver,
    mut interval: Option<Duration>,
    snapshot: &dyn Fn() -> SaveSnapshot,
) {
    let now_ms = || unix_ms(SystemTime::now());
    let mut next_tick = next_deadline(Instant::now(), interval);
    loop {
        let received = match next_tick {
            Some(deadline) => rx.recv_deadline(deadline),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(msg) => {
                if let Msg::SetInterval(new) = msg {
                    interval = new;
                    next_tick = next_deadline(Instant::now(), interval);
                }
                if saver.handle(msg, snapshot, now_ms(), store) == Flow::Exit {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                next_tick = next_deadline(Instant::now(), interval);
                // `saved_at_ms` is the time the snapshot was taken.
                let (snap, now) = (snapshot(), now_ms());
                if let Err(err) = saver.tick(&snap, now, store) {
                    tracing::warn!(%err, "autosave failed; retrying at the next interval");
                }
            }
            // Every handle was dropped: the app is gone without a clean exit. Keep the files.
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// What was last written: (document generation, history position).
type SaveKey = (u64, HistoryPosition);

/// The autosave decisions, without threads or clocks.
#[derive(Debug)]
pub struct Autosaver {
    last_written: Option<SaveKey>,
    /// Held for the process lifetime; dropped before the lock file is deleted on `Shutdown`.
    lock: Option<File>,
}

impl Autosaver {
    pub fn new(lock: Option<File>) -> Self {
        Self {
            last_written: None,
            lock,
        }
    }

    /// One interval elapsed. Writes `snap` if it is dirty and differs from the last write; deletes
    /// the own autosave once the project is clean again (e.g. undone back to the saved state).
    /// Returns whether it wrote. On failure nothing is recorded, so the next tick retries.
    pub fn tick(
        &mut self,
        snap: &SaveSnapshot,
        now_ms: i64,
        store: &RecoveryStore,
    ) -> io::Result<bool> {
        if !snap.dirty {
            if self.last_written.is_some() {
                store.delete_own()?;
                self.last_written = None;
            }
            return Ok(false);
        }
        // During a gesture the key is its base's, so the base is written once, not per update.
        if self.last_written == Some(snap.key()) {
            return Ok(false);
        }
        self.write(snap, now_ms, store)?;
        Ok(true)
    }

    pub fn handle(
        &mut self,
        msg: Msg,
        snapshot: &dyn Fn() -> SaveSnapshot,
        now_ms: i64,
        store: &RecoveryStore,
    ) -> Flow {
        match msg {
            Msg::SetInterval(Some(_)) => Flow::Continue,
            Msg::Discard | Msg::SetInterval(None) => {
                self.last_written = None;
                if let Err(err) = store.delete_own() {
                    tracing::warn!(%err, "failed to delete the autosave");
                }
                Flow::Continue
            }
            Msg::Adopt { old } => {
                // The old entry is deleted only once its content is safe in the own autosave.
                match self.write(&snapshot(), now_ms, store) {
                    Ok(()) => {
                        if let Err(err) = store.delete_adopted(old) {
                            tracing::warn!(%old, %err, "failed to delete a restored autosave");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%old, %err, "failed to autosave a restored project");
                    }
                }
                Flow::Continue
            }
            Msg::Shutdown { ack } => {
                drop(self.lock.take());
                if let Err(err) = store.delete_own_all() {
                    tracing::warn!(%err, "failed to delete the autosave on exit");
                }
                let _ = ack.send(());
                Flow::Exit
            }
        }
    }

    fn write(&mut self, snap: &SaveSnapshot, now_ms: i64, store: &RecoveryStore) -> io::Result<()> {
        store.write(snap, now_ms)?;
        self.last_written = Some(snap.key());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use app_core::Project;
    use app_core::command::RenameProject;
    use app_core::serialization::{self, CURRENT_SCHEMA_VERSION};

    use super::*;
    use crate::recovery::tests::store;
    use crate::state::AppState;

    fn rename(state: &AppState, name: &str) {
        state.test_edit(|doc| doc.execute(Box::new(RenameProject::new(name))).map(Some));
    }

    fn gesture(state: &AppState, name: &str) {
        state.test_edit(|doc| {
            doc.gesture_update(Box::new(RenameProject::new(name)))
                .map(Some)
        });
    }

    fn own_file(store: &RecoveryStore) -> PathBuf {
        PathBuf::from(format!("{}.json", store.session_id()))
    }

    fn written_name(dir: &Path, store: &RecoveryStore) -> Option<String> {
        let text = fs::read_to_string(dir.join(own_file(store))).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        Some(value["project"]["name"].as_str().unwrap().to_owned())
    }

    struct Fixture {
        dir: tempfile::TempDir,
        state: AppState,
        saver: Autosaver,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state = AppState::new(
                Project::new("Start"),
                None,
                store(dir.path()),
                crate::jobs::JobManager::for_tests(),
                std::sync::Arc::new(crate::previews::Previews::new(None)),
            );
            Self {
                dir,
                state,
                saver: Autosaver::new(None),
            }
        }

        fn tick(&mut self) -> bool {
            self.saver
                .tick(&self.state.save_snapshot(), 42, self.state.recovery())
                .unwrap()
        }

        fn handle(&mut self, msg: Msg) -> Flow {
            let state = &self.state;
            self.saver
                .handle(msg, &|| state.save_snapshot(), 42, state.recovery())
        }

        fn written(&self) -> Option<String> {
            written_name(self.dir.path(), self.state.recovery())
        }
    }

    #[test]
    fn writes_only_when_dirty_and_changed() {
        let mut f = Fixture::new();
        assert!(!f.tick(), "clean");
        assert_eq!(f.written(), None);

        rename(&f.state, "A");
        assert!(f.tick());
        assert_eq!(f.written().as_deref(), Some("A"));
        assert!(!f.tick(), "unchanged key");

        rename(&f.state, "B");
        assert!(f.tick(), "written again after an edit");
        assert_eq!(f.written().as_deref(), Some("B"));
    }

    #[test]
    fn writes_the_gesture_base_once() {
        let mut f = Fixture::new();
        rename(&f.state, "Base");
        gesture(&f.state, "Live 1");
        assert!(f.tick());
        assert_eq!(f.written().as_deref(), Some("Base"));
        gesture(&f.state, "Live 2");
        assert!(!f.tick(), "gesture updates keep the base's key");
        assert_eq!(f.written().as_deref(), Some("Base"));
    }

    #[test]
    fn a_failed_write_is_retried_on_the_next_tick() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("recovery");
        // A file where the directory should be: every write fails.
        fs::write(&blocked, "not a directory").unwrap();
        let state = AppState::new(
            Project::new("Start"),
            None,
            store(&blocked),
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        );
        let mut saver = Autosaver::new(None);
        rename(&state, "A");
        assert!(
            saver
                .tick(&state.save_snapshot(), 1, state.recovery())
                .is_err()
        );

        fs::remove_file(&blocked).unwrap();
        fs::create_dir(&blocked).unwrap();
        assert!(
            saver
                .tick(&state.save_snapshot(), 2, state.recovery())
                .unwrap()
        );
        assert_eq!(
            written_name(&blocked, state.recovery()).as_deref(),
            Some("A")
        );
    }

    #[test]
    fn returning_to_the_saved_state_deletes_the_autosave() {
        let mut f = Fixture::new();
        rename(&f.state, "A");
        assert!(f.tick());
        f.state.test_edit(|doc| Ok(doc.undo().then_some(())));
        assert!(!f.tick());
        assert_eq!(f.written(), None);
    }

    #[test]
    fn discard_deletes_and_the_next_dirty_tick_rewrites() {
        let mut f = Fixture::new();
        rename(&f.state, "A");
        assert!(f.tick());
        assert_eq!(f.handle(Msg::Discard), Flow::Continue);
        assert_eq!(f.written(), None);
        assert!(f.tick(), "still dirty, same key, but the file is gone");
        assert_eq!(f.written().as_deref(), Some("A"));
    }

    #[test]
    fn a_save_discards_and_a_clean_project_is_not_rewritten() {
        let mut f = Fixture::new();
        rename(&f.state, "A");
        assert!(f.tick());
        let saved = f.state.save_snapshot();
        f.state
            .test_mark_saved(&saved, PathBuf::from("/p/a.cutframe"));
        f.handle(Msg::Discard);
        assert!(!f.tick());
        assert_eq!(f.written(), None);
    }

    #[test]
    fn adopt_writes_the_own_file_then_deletes_the_old_entry() {
        let mut f = Fixture::new();
        let other = RecoveryStore::new(f.dir.path().to_path_buf(), Uuid::now_v7());
        drop(other.acquire_lock().unwrap());
        rename(&f.state, "Crashed");
        other.write(&f.state.save_snapshot(), 7).unwrap();

        let restored = f.state.recovery().load(other.session_id()).unwrap();
        f.state.test_replace_restored(
            restored.project,
            restored.file_path,
            restored.source_version,
        );
        assert_eq!(
            f.handle(Msg::Adopt {
                old: other.session_id()
            }),
            Flow::Continue
        );

        assert_eq!(f.written().as_deref(), Some("Crashed"));
        let mut names: Vec<_> = fs::read_dir(f.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [own_file(f.state.recovery()).into_os_string()]);
        assert!(!f.tick(), "already written");
    }

    #[test]
    fn shutdown_deletes_the_file_and_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            Project::new("Start"),
            None,
            store(dir.path()),
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        );
        let lock = state.recovery().acquire_lock().unwrap();
        let mut saver = Autosaver::new(Some(lock));
        rename(&state, "A");
        assert!(
            saver
                .tick(&state.save_snapshot(), 1, state.recovery())
                .unwrap()
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);

        let (ack, done) = crossbeam_channel::bounded(1);
        let flow = saver.handle(
            Msg::Shutdown { ack },
            &|| state.save_snapshot(),
            2,
            state.recovery(),
        );
        assert_eq!(flow, Flow::Exit);
        done.try_recv().expect("acknowledged");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_thread_stops_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let lock = s.acquire_lock().unwrap();
        let handle = spawn(s, Some(lock), Some(Duration::from_secs(60)), || {
            AppState::new(
                Project::new("P"),
                None,
                store(Path::new("unused")),
                crate::jobs::JobManager::for_tests(),
                std::sync::Arc::new(crate::previews::Previews::new(None)),
            )
            .save_snapshot()
        })
        .unwrap();
        handle.shutdown();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn next_deadline_is_one_interval_away_or_never() {
        let now = Instant::now();
        assert_eq!(
            next_deadline(now, Some(Duration::from_secs(15))),
            Some(now + Duration::from_secs(15))
        );
        assert_eq!(next_deadline(now, None), None);
    }

    #[test]
    fn switching_autosave_off_deletes_the_own_file() {
        let mut f = Fixture::new();
        rename(&f.state, "A");
        assert!(f.tick());
        assert_eq!(
            f.handle(Msg::SetInterval(Some(Duration::from_secs(30)))),
            Flow::Continue
        );
        assert_eq!(f.written().as_deref(), Some("A"), "a new interval keeps it");
        assert_eq!(f.handle(Msg::SetInterval(None)), Flow::Continue);
        assert_eq!(f.written(), None);
    }

    /// The thread waits for the new, shorter deadline instead of the old one; with autosave off
    /// it never ticks.
    #[test]
    fn the_thread_follows_interval_changes() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            Project::new("Start"),
            None,
            store(dir.path()),
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        );
        rename(&state, "Dirty");
        let snap = state.save_snapshot();
        let handle = spawn(state.recovery().clone(), None, None, move || snap.clone()).unwrap();
        let own = dir.path().join(own_file(state.recovery()));

        thread::sleep(Duration::from_millis(100));
        assert!(!own.exists(), "off: no ticks");

        handle.send(Msg::SetInterval(Some(Duration::from_secs(3_600))));
        handle.send(Msg::SetInterval(Some(Duration::from_millis(20))));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !own.exists() {
            assert!(Instant::now() < deadline, "no tick at the new deadline");
            thread::sleep(Duration::from_millis(10));
        }

        handle.send(Msg::SetInterval(None));
        let deadline = Instant::now() + Duration::from_secs(5);
        while own.exists() {
            assert!(Instant::now() < deadline, "off did not delete the autosave");
            thread::sleep(Duration::from_millis(10));
        }
        handle.shutdown();
    }

    /// Design budget (autosave-recovery §10): one tick on a 1,000-clip, 20-track project holds the
    /// document lock < 1 ms and writes in < 50 ms. Run with
    /// `cargo test --release -p cutframe -- --ignored`.
    #[test]
    #[ignore = "timing test; run in release mode"]
    fn a_large_project_autosaves_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let project = crate::project_file::tests::large_project();
        let state = AppState::new(
            Project::new("Start"),
            None,
            store(dir.path()),
            crate::jobs::JobManager::for_tests(),
            std::sync::Arc::new(crate::previews::Previews::new(None)),
        );
        state.test_replace(project.clone(), None, CURRENT_SCHEMA_VERSION);
        rename(&state, "Large");
        let mut saver = Autosaver::new(None);

        let started = Instant::now();
        let snap = state.save_snapshot();
        let locked = started.elapsed();
        let started = Instant::now();
        assert!(saver.tick(&snap, 1, state.recovery()).unwrap());
        let written = started.elapsed();

        assert!(
            locked < Duration::from_millis(1),
            "snapshot took {locked:?}"
        );
        assert!(
            written < Duration::from_millis(50),
            "write took {written:?}"
        );
        let restored = RecoveryStore::new(dir.path().to_path_buf(), Uuid::now_v7())
            .load(state.recovery().session_id())
            .unwrap();
        assert_eq!(
            serialization::to_json(&restored.project),
            serialization::to_json(&snap.project)
        );
    }
}
