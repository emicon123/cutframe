//! The shared job manager: a bounded worker pool for work that can take longer than ~100 ms
//! (import probes now; previews, relink and export later), ADR-0003 §8, ADR-0004, design §6.1.
//!
//! - `P = clamp(available_parallelism / 4, 1, 3)` worker threads serve two FIFO queues; the
//!   interactive queue always goes before the background queue.
//! - A job is split into tasks. The registry maps each running [`JobId`] to its cancel flag and
//!   progress counter; a ticker emits `job-progress` at most 10 Hz per job, only when it moved.
//! - A job ends with [`JobContext::finish`], which emits `job-finished` exactly once. No
//!   `job-progress` for a job is emitted after its `job-finished`.
//! - Jobs reach the model only through [`JobHost::edit`] (a normal command under the short
//!   document lock, ADR-0004 §1). Events go through the host too, so tests run without a window.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use app_core::command::CommandError;
use app_core::{Document, MediaId, Project};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::ipc::IpcError;
use crate::ipc::events::{JobFinished, JobProgress, MediaJobReport};
use crate::ipc::types::{JobId, JobStage};
use crate::media_status::StatusUpdate;
use crate::state::AppState;

/// Minimum time between two `job-progress` events of one job.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// What jobs need from the app: events to the UI and edits of the document.
pub trait JobHost: Send + Sync + 'static {
    fn job_progress(&self, event: JobProgress);
    fn job_finished(&self, event: JobFinished);
    fn media_job_report(&self, event: MediaJobReport);
    /// Runs `edit` on the document like a command handler does: under the document lock,
    /// publishing the snapshot and emitting `project-changed` if `edit` returns `Ok(true)`.
    fn edit(
        &self,
        edit: &mut dyn FnMut(&mut Document) -> Result<bool, CommandError>,
    ) -> Result<(), IpcError>;
    /// The latest published project (lock-free for the real host).
    fn snapshot(&self) -> Arc<Project>;
    /// Media was added to the project (an import finished): start generating its previews.
    fn media_added(&self, ids: Vec<MediaId>);
    /// Records what a media job found out about files (session state, design §6.6); emits
    /// `project-changed` when a status changed.
    fn media_status(&self, updates: Vec<StatusUpdate>);
}

/// How often [`edit_when_idle`] retries while a gesture is open.
const GESTURE_RETRY: Duration = Duration::from_millis(50);

/// Runs `edit` through the host once no gesture is open (ADR-0006 §3: a job's edit never lands
/// inside a drag), retrying every 50 ms. `edit` returns its result and whether it changed the
/// document. `Ok(None)` when the job was cancelled while waiting.
pub fn edit_when_idle<T>(
    ctx: &JobContext,
    mut edit: impl FnMut(&mut Document) -> Result<(T, bool), CommandError>,
) -> Result<Option<T>, IpcError> {
    loop {
        let mut result = None;
        ctx.host().edit(&mut |doc| {
            if doc.gesture_label().is_some() {
                return Ok(false);
            }
            let (value, changed) = edit(doc)?;
            result = Some(value);
            Ok(changed)
        })?;
        if result.is_some() {
            return Ok(result);
        }
        if ctx.is_cancelled() {
            return Ok(None);
        }
        std::thread::sleep(GESTURE_RETRY);
    }
}

/// Which queue a task goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Work the user waits for: import probes, `media://` misses, relink.
    Interactive,
    /// Prefetch the user does not wait for: preview generation, media checks (0240).
    Background,
}

type Task = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queues {
    interactive: VecDeque<Task>,
    background: VecDeque<Task>,
}

/// Work done out of the total, shared by a job's tasks and the ticker.
#[derive(Debug, Default)]
struct Progress {
    done: AtomicUsize,
    /// 0 while the amount of work is not known yet.
    total: AtomicUsize,
}

struct JobEntry {
    stage: JobStage,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    /// (done, total) of the last `job-progress`, to emit only on change.
    reported: Option<(usize, usize)>,
}

struct Inner {
    host: Arc<dyn JobHost>,
    queues: Mutex<Queues>,
    work_available: Condvar,
    jobs: Mutex<HashMap<JobId, JobEntry>>,
    jobs_changed: Condvar,
    shutdown: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Tasks never panic while holding these locks (they run outside them); recover anyway.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The job manager. Dropping it stops its threads after their current task.
pub struct JobManager {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for JobManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobManager").finish_non_exhaustive()
    }
}

/// `clamp(available_parallelism / 4, 1, 3)`: with 2 decoder threads per task the pool uses at most
/// about 6 cores and leaves room for playback (design §6.1).
pub fn pool_size() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 4).clamp(1, 3)
}

impl JobManager {
    /// A job manager whose events go nowhere, for tests that need an `AppState`.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        let (host, _rx) = tests::FakeHost::new();
        Self::new(host)
    }

    pub fn new(host: Arc<dyn JobHost>) -> Self {
        Self::with_workers(host, pool_size())
    }

    pub fn with_workers(host: Arc<dyn JobHost>, workers: usize) -> Self {
        let inner = Arc::new(Inner {
            host,
            queues: Mutex::default(),
            work_available: Condvar::new(),
            jobs: Mutex::default(),
            jobs_changed: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        for n in 0..workers.max(1) {
            let inner = Arc::clone(&inner);
            std::thread::Builder::new()
                .name(format!("job-worker-{n}"))
                .spawn(move || worker(&inner))
                .expect("spawn a job worker");
        }
        let ticker = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("job-progress".into())
            .spawn(move || progress_ticker(&ticker))
            .expect("spawn the job progress ticker");
        Self { inner }
    }

    /// Registers a new job and queues `first` as its first task. Returns at once.
    pub fn start(
        &self,
        stage: JobStage,
        priority: Priority,
        first: impl FnOnce(JobContext) + Send + 'static,
    ) -> JobId {
        let id = JobId::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        lock(&self.inner.jobs).insert(
            id,
            JobEntry {
                stage,
                cancel: Arc::clone(&cancel),
                progress: Arc::clone(&progress),
                reported: None,
            },
        );
        self.inner.jobs_changed.notify_all();
        let context = JobContext {
            id,
            cancel,
            progress,
            inner: Arc::clone(&self.inner),
        };
        context.spawn(priority, first);
        id
    }

    /// A handle that queues tasks that belong to no job (no events), e.g. `media://` misses.
    pub fn spawner(&self) -> JobSpawner {
        JobSpawner {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Asks job `id` to stop. Idempotent: unknown or finished ids are ignored.
    pub fn cancel(&self, id: JobId) {
        if let Some(job) = lock(&self.inner.jobs).get(&id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);
        // Take each lock before notifying, so a thread between its check and its wait still
        // gets the wake-up.
        drop(lock(&self.inner.queues));
        self.inner.work_available.notify_all();
        drop(lock(&self.inner.jobs));
        self.inner.jobs_changed.notify_all();
    }
}

/// Queues tasks on the pool that belong to no job: no progress, no `job-finished`. Cheap to
/// clone; tasks queued after the manager is dropped never run.
#[derive(Clone)]
pub struct JobSpawner {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for JobSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobSpawner").finish_non_exhaustive()
    }
}

impl JobSpawner {
    pub fn run(&self, priority: Priority, task: impl FnOnce() + Send + 'static) {
        push(&self.inner, priority, Box::new(task));
    }
}

fn push(inner: &Inner, priority: Priority, task: Task) {
    {
        let mut queues = lock(&inner.queues);
        match priority {
            Priority::Interactive => queues.interactive.push_back(task),
            Priority::Background => queues.background.push_back(task),
        }
    }
    inner.work_available.notify_one();
}

/// What a job's tasks get: its id, cancel flag and progress, and a way to queue more tasks.
#[derive(Clone)]
pub struct JobContext {
    id: JobId,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for JobContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobContext")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The flag to hand to FFmpeg's interrupt callback.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    pub fn host(&self) -> &dyn JobHost {
        &*self.inner.host
    }

    /// Sets the amount of work; progress is `done / total` from then on.
    pub fn set_total(&self, total: usize) {
        self.progress.total.store(total, Ordering::Relaxed);
    }

    /// Counts one unit of work as done.
    pub fn step_done(&self) {
        self.progress.done.fetch_add(1, Ordering::Relaxed);
    }

    /// Queues another task of this job.
    pub fn spawn(&self, priority: Priority, task: impl FnOnce(JobContext) + Send + 'static) {
        let context = self.clone();
        push(&self.inner, priority, Box::new(move || task(context)));
    }

    /// Ends the job: unregisters it, then emits `job-finished`. Emit everything else first.
    pub fn finish(self, result: Result<(), IpcError>) {
        lock(&self.inner.jobs).remove(&self.id);
        self.inner.host.job_finished(JobFinished {
            job_id: self.id,
            result,
        });
    }
}

fn worker(inner: &Inner) {
    loop {
        let task = {
            let mut queues = lock(&inner.queues);
            loop {
                if inner.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(task) = queues
                    .interactive
                    .pop_front()
                    .or_else(|| queues.background.pop_front())
                {
                    break task;
                }
                queues = inner
                    .work_available
                    .wait(queues)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        // A panicking task must not take the worker down with it. Tasks that own a job's
        // completion catch their own panics so the job still finishes.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)).is_err() {
            tracing::error!("a job task panicked");
        }
    }
}

fn progress_ticker(inner: &Inner) {
    loop {
        {
            let mut jobs = lock(&inner.jobs);
            while jobs.is_empty() {
                if inner.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                jobs = inner
                    .jobs_changed
                    .wait(jobs)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            // Emitted under the registry lock, so `finish` (which unregisters first) can never be
            // overtaken by a late progress event of the same job.
            for (&id, job) in jobs.iter_mut() {
                let done = job.progress.done.load(Ordering::Relaxed);
                let total = job.progress.total.load(Ordering::Relaxed);
                if job.reported == Some((done, total)) {
                    continue;
                }
                job.reported = Some((done, total));
                inner.host.job_progress(JobProgress {
                    job_id: id,
                    stage: job.stage,
                    progress: (total > 0).then(|| (done as f32 / total as f32).min(1.0)),
                });
            }
        }
        if inner.shutdown.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(PROGRESS_INTERVAL);
    }
}

/// The real host: Tauri events and the app's [`AppState`].
#[derive(Debug)]
pub struct TauriJobHost {
    app: AppHandle,
}

impl TauriJobHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl JobHost for TauriJobHost {
    fn job_progress(&self, event: JobProgress) {
        if let Err(err) = event.emit(&self.app) {
            tracing::warn!(%err, "failed to emit job-progress");
        }
    }

    fn job_finished(&self, event: JobFinished) {
        if let Err(err) = event.emit(&self.app) {
            tracing::warn!(%err, "failed to emit job-finished");
        }
    }

    fn media_job_report(&self, event: MediaJobReport) {
        if let Err(err) = event.emit(&self.app) {
            tracing::warn!(%err, "failed to emit media-job-report");
        }
    }

    fn edit(
        &self,
        edit: &mut dyn FnMut(&mut Document) -> Result<bool, CommandError>,
    ) -> Result<(), IpcError> {
        let state = self.app.state::<AppState>();
        state.edit(&self.app, |doc| {
            edit(doc).map(|changed| changed.then_some(()))
        })?;
        Ok(())
    }

    fn snapshot(&self) -> Arc<Project> {
        self.app.state::<AppState>().snapshot()
    }

    fn media_added(&self, ids: Vec<MediaId>) {
        self.app.state::<AppState>().prefetch_previews(Some(ids));
    }

    fn media_status(&self, updates: Vec<StatusUpdate>) {
        if let Err(err) = self
            .app
            .state::<AppState>()
            .set_media_status(&self.app, updates)
        {
            tracing::warn!(%err, "failed to emit project-changed for media status");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;
    use crate::media_status::MediaStatuses;

    /// Records events and edits a plain document, like `AppState` without a window.
    #[derive(Debug)]
    pub(crate) struct FakeHost {
        pub document: Mutex<Document>,
        pub events: Mutex<Vec<Recorded>>,
        pub changes: AtomicUsize,
        /// Ids passed to `media_added`, one entry per call.
        pub added: Mutex<Vec<Vec<MediaId>>>,
        /// Media status as `AppState` would hold it.
        pub statuses: Mutex<MediaStatuses>,
        finished: Mutex<Option<mpsc::Sender<JobId>>>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum Recorded {
        Progress(JobProgress),
        Finished(JobFinished),
        Report(MediaJobReport),
    }

    impl FakeHost {
        pub(crate) fn new() -> (Arc<Self>, mpsc::Receiver<JobId>) {
            let (tx, rx) = mpsc::channel();
            let host = Arc::new(Self {
                document: Mutex::new(Document::new(Project::new("Test"))),
                events: Mutex::default(),
                changes: AtomicUsize::new(0),
                added: Mutex::default(),
                statuses: Mutex::default(),
                finished: Mutex::new(Some(tx)),
            });
            (host, rx)
        }

        pub(crate) fn events(&self) -> Vec<Recorded> {
            lock(&self.events).clone()
        }
    }

    impl JobHost for FakeHost {
        fn job_progress(&self, event: JobProgress) {
            lock(&self.events).push(Recorded::Progress(event));
        }

        fn job_finished(&self, event: JobFinished) {
            let id = event.job_id;
            lock(&self.events).push(Recorded::Finished(event));
            if let Some(tx) = &*lock(&self.finished) {
                tx.send(id).ok();
            }
        }

        fn media_job_report(&self, event: MediaJobReport) {
            lock(&self.events).push(Recorded::Report(event));
        }

        fn edit(
            &self,
            edit: &mut dyn FnMut(&mut Document) -> Result<bool, CommandError>,
        ) -> Result<(), IpcError> {
            if edit(&mut lock(&self.document))? {
                self.changes.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }

        fn snapshot(&self) -> Arc<Project> {
            lock(&self.document).snapshot()
        }

        fn media_added(&self, ids: Vec<MediaId>) {
            lock(&self.added).push(ids);
        }

        fn media_status(&self, updates: Vec<StatusUpdate>) {
            lock(&self.statuses).apply(updates);
        }
    }

    pub(crate) fn wait_finished(rx: &mpsc::Receiver<JobId>) -> JobId {
        rx.recv_timeout(Duration::from_secs(30))
            .expect("job finished in time")
    }

    #[test]
    fn pool_size_is_bounded() {
        assert!((1..=3).contains(&pool_size()));
    }

    #[test]
    fn a_job_reports_progress_then_finishes_once() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 2);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let id = jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
            ctx.set_total(2);
            ctx.step_done();
            // Hold the job open until the ticker has seen half the work.
            lock(&release_rx).recv().ok();
            ctx.step_done();
            ctx.finish(Ok(()));
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !host
            .events()
            .iter()
            .any(|e| matches!(e, Recorded::Progress(p) if p.progress == Some(0.5)))
        {
            assert!(Instant::now() < deadline, "no progress event");
            std::thread::sleep(Duration::from_millis(10));
        }
        release_tx.send(()).unwrap();
        assert_eq!(wait_finished(&rx), id);
        // Give a late tick the chance to (wrongly) appear.
        std::thread::sleep(PROGRESS_INTERVAL * 2);

        let events = host.events();
        let finished: Vec<_> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Recorded::Finished(_)))
            .collect();
        assert_eq!(finished.len(), 1);
        assert_eq!(
            finished[0].1,
            &Recorded::Finished(JobFinished {
                job_id: id,
                result: Ok(())
            })
        );
        assert_eq!(
            finished[0].0,
            events.len() - 1,
            "nothing after job-finished"
        );
        assert!(events.iter().all(|e| match e {
            Recorded::Progress(p) => p.job_id == id && p.stage == JobStage::Probing,
            _ => true,
        }));
    }

    #[test]
    fn interactive_tasks_run_before_background_tasks() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        let order = Arc::new(Mutex::new(Vec::new()));
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Arc::new(Mutex::new(gate_rx));
        let log = Arc::clone(&order);
        jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
            // The only worker is busy here while both queues fill up.
            lock(&gate_rx).recv().ok();
            let remaining = Arc::new(AtomicUsize::new(4));
            for (priority, name) in [
                (Priority::Background, "background 1"),
                (Priority::Interactive, "interactive 1"),
                (Priority::Background, "background 2"),
                (Priority::Interactive, "interactive 2"),
            ] {
                let log = Arc::clone(&log);
                let remaining = Arc::clone(&remaining);
                ctx.spawn(priority, move |ctx| {
                    lock(&log).push(name);
                    if remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
                        ctx.finish(Ok(()));
                    }
                });
            }
        });
        gate_tx.send(()).unwrap();
        wait_finished(&rx);
        assert_eq!(
            *lock(&order),
            [
                "interactive 1",
                "interactive 2",
                "background 1",
                "background 2"
            ]
        );
    }

    #[test]
    fn cancel_sets_the_flag_and_ignores_unknown_ids() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 1);
        jobs.cancel(JobId::new());
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let id = jobs.start(JobStage::Probing, Priority::Interactive, move |ctx| {
            started_tx.send(()).ok();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ctx.is_cancelled() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            let result = if ctx.is_cancelled() {
                Err(IpcError::Cancelled("cancelled".into()))
            } else {
                Ok(())
            };
            ctx.finish(result);
        });
        started_rx.recv().unwrap();
        jobs.cancel(id);
        wait_finished(&rx);
        jobs.cancel(id); // finished: ignored
        assert!(host.events().contains(&Recorded::Finished(JobFinished {
            job_id: id,
            result: Err(IpcError::Cancelled("cancelled".into())),
        })));
    }
}
