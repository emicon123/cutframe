//! The session's export encoder probe and its cache behind `get_export_capabilities`
//! (export.md §4.4).
//!
//! The probe runs once per session, on the first `get_export_capabilities` (the export dialog
//! opening), never at app start: it wakes the discrete GPU of a hybrid laptop. It is a short job
//! (`probingEncoders`) on its own thread, so it does not hold a pool worker for 1-3 s, and it
//! ends with `export-capabilities-changed` and `job-finished`. It ignores `cancel_job`.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use engine::export::encoders::{EncoderFilter, ProbeReport};

use crate::ipc::events::ExportCapabilitiesChanged;
use crate::ipc::types::{ExportCapabilities, JobStage};
use crate::jobs::{JobManager, ThreadPriority};

/// Runs the probe; [`ProbeReport::run`] outside tests.
type Runner = fn(&EncoderFilter) -> ProbeReport;

#[derive(Debug)]
enum ProbeState {
    NotStarted,
    Running,
    Done(Arc<ProbeReport>),
}

/// The session's probe result, shared by the commands and the probe job.
#[derive(Debug)]
pub struct EncoderProbe {
    state: Mutex<ProbeState>,
    finished: Condvar,
    runner: Runner,
}

impl Default for EncoderProbe {
    fn default() -> Self {
        Self::with_runner(ProbeReport::run)
    }
}

fn lock(state: &Mutex<ProbeState>) -> MutexGuard<'_, ProbeState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl EncoderProbe {
    pub fn with_runner(runner: Runner) -> Self {
        Self {
            state: Mutex::new(ProbeState::NotStarted),
            finished: Condvar::new(),
            runner,
        }
    }

    /// The cached capabilities. The first call of a session starts the probe and returns
    /// [`ExportCapabilities::PENDING`]; `export-capabilities-changed` follows. Never blocks.
    pub fn capabilities(self: &Arc<Self>, jobs: &JobManager) -> ExportCapabilities {
        match self.report_or_start(jobs) {
            Some(report) => ExportCapabilities::from(&*report),
            None => ExportCapabilities::PENDING,
        }
    }

    /// The finished report, or `None` (and the probe started if it was not yet).
    pub fn report_or_start(self: &Arc<Self>, jobs: &JobManager) -> Option<Arc<ProbeReport>> {
        let mut state = lock(&self.state);
        match &*state {
            ProbeState::Done(report) => return Some(Arc::clone(report)),
            ProbeState::Running => return None,
            ProbeState::NotStarted => *state = ProbeState::Running,
        }
        drop(state);
        self.start(jobs);
        None
    }

    /// Waits for the report of a probe that [`Self::report_or_start`] started (the export job,
    /// when `start_export` came before the dialog's probe finished). `None` once `cancelled`
    /// returns true, or at once if the probe was never started.
    pub fn wait(&self, cancelled: impl Fn() -> bool) -> Option<Arc<ProbeReport>> {
        /// How often the cancel condition is checked.
        const STEP: Duration = Duration::from_millis(50);
        let mut state = lock(&self.state);
        loop {
            match &*state {
                ProbeState::Done(report) => return Some(Arc::clone(report)),
                ProbeState::NotStarted => return None,
                ProbeState::Running => {}
            }
            if cancelled() {
                return None;
            }
            state = self
                .finished
                .wait_timeout(state, STEP)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    fn start(self: &Arc<Self>, jobs: &JobManager) {
        let probe = Arc::clone(self);
        jobs.start_dedicated(
            JobStage::ProbingEncoders,
            "encoder-probe",
            ThreadPriority::Normal,
            move |ctx| {
                let filter = EncoderFilter::from_env();
                let runner = probe.runner;
                let report = std::panic::catch_unwind(|| runner(&filter)).unwrap_or_else(|_| {
                    tracing::error!("the encoder probe panicked; no export encoder is available");
                    ProbeReport {
                        entries: Vec::new(),
                    }
                });
                let report = Arc::new(report);
                *lock(&probe.state) = ProbeState::Done(Arc::clone(&report));
                probe.finished.notify_all();
                ctx.host()
                    .export_capabilities_changed(ExportCapabilitiesChanged(
                        ExportCapabilities::from(&*report),
                    ));
                ctx.finish(Ok(()));
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use engine::export::encoders::{EncoderKind, OptionSet};

    use super::*;
    use crate::jobs::tests::{FakeHost, Recorded, wait_finished};

    /// Runs of [`counting_runner`] (used by one test only).
    static RUNS: AtomicUsize = AtomicUsize::new(0);

    fn counting_runner(filter: &EncoderFilter) -> ProbeReport {
        RUNS.fetch_add(1, Ordering::SeqCst);
        fake_runner(filter)
    }

    /// OpenH264 only, after a short delay (so the probe is still running for a while).
    pub(crate) fn fake_runner(filter: &EncoderFilter) -> ProbeReport {
        std::thread::sleep(Duration::from_millis(100));
        ProbeReport::run_with(filter, |kind, _| match kind {
            EncoderKind::OpenH264 => Ok(OptionSet::Full),
            _ => Err(engine::export::encoders::EncoderError::NotInBuild("fake")),
        })
    }

    fn panicking_runner(_: &EncoderFilter) -> ProbeReport {
        panic!("probe panic")
    }

    #[test]
    fn the_first_call_starts_the_probe_once_and_the_event_carries_the_result() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host.clone(), 1);
        let probe = Arc::new(EncoderProbe::with_runner(counting_runner));

        assert_eq!(probe.capabilities(&jobs), ExportCapabilities::PENDING);
        assert_eq!(probe.capabilities(&jobs), ExportCapabilities::PENDING);
        wait_finished(&rx);
        let caps = probe.capabilities(&jobs);
        assert!(caps.probed);
        assert_eq!(caps.encoders.len(), 10);
        assert_eq!(caps.encoders.iter().filter(|e| e.available).count(), 1);
        assert_eq!(RUNS.load(Ordering::SeqCst), 1, "probed once");

        let events = host.events();
        let changed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Recorded::Capabilities(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(changed, [ExportCapabilitiesChanged(caps)]);
        // The event comes before the job's end; progress carries the probe's stage.
        let changed_at = events
            .iter()
            .position(|e| matches!(e, Recorded::Capabilities(_)));
        let finished_at = events
            .iter()
            .position(|e| matches!(e, Recorded::Finished(f) if f.result.is_ok()));
        assert!(changed_at < finished_at);
        assert!(events.iter().all(|e| match e {
            Recorded::Progress(p) => p.stage == JobStage::ProbingEncoders,
            _ => true,
        }));
    }

    #[test]
    fn wait_returns_the_report_of_the_started_probe() {
        let (host, _rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        let probe = Arc::new(EncoderProbe::with_runner(fake_runner));
        assert!(probe.wait(|| false).is_none(), "not started");
        assert!(probe.report_or_start(&jobs).is_none());
        let report = probe.wait(|| false).expect("a report");
        assert_eq!(
            report
                .candidates(engine::export::plan::VideoCodec::H264, None)
                .len(),
            1
        );
        // Done: cached, no second probe.
        assert!(probe.wait(|| true).is_some());
    }

    #[test]
    fn wait_gives_up_when_cancelled() {
        let (host, _rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        let probe = Arc::new(EncoderProbe::with_runner(fake_runner));
        probe.report_or_start(&jobs);
        assert!(probe.wait(|| true).is_none());
    }

    #[test]
    fn a_panicking_probe_reports_no_encoders() {
        let (host, rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        let probe = Arc::new(EncoderProbe::with_runner(panicking_runner));
        probe.capabilities(&jobs);
        wait_finished(&rx);
        assert_eq!(
            probe.capabilities(&jobs),
            ExportCapabilities {
                probed: true,
                encoders: Vec::new()
            }
        );
    }
}
