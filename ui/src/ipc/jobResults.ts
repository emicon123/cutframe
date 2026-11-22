import type { IpcError, JobFinished, JobId, MediaFileResult, MediaJobReport } from "@/bindings";

/** How a job ended: its per-file report (empty without one) and its `job-finished` result. */
export type JobOutcome = { files: MediaFileResult[]; error: IpcError | null };

/** How many finished jobs are remembered for a `waitForJob` that comes late. */
const MEMORY = 32;

/**
 * Lets a feature that starts a job await its end by id. Events can arrive before the command that
 * started the job resolves, so finished results are remembered for a while. Fed by `jobSync`.
 */
export function createJobResults() {
  const reports = new Map<JobId, MediaFileResult[]>();
  const finished = new Map<JobId, JobOutcome>();
  const waiters = new Map<JobId, ((outcome: JobOutcome) => void)[]>();

  return {
    report(event: MediaJobReport): void {
      reports.set(event.jobId, event.files);
    },
    finished(event: JobFinished): void {
      const outcome: JobOutcome = { files: reports.get(event.jobId) ?? [], error: event.result.Err ?? null };
      reports.delete(event.jobId);
      const waiting = waiters.get(event.jobId);
      if (waiting) {
        waiters.delete(event.jobId);
        for (const resolve of waiting) resolve(outcome);
        return;
      }
      finished.set(event.jobId, outcome);
      if (finished.size > MEMORY) {
        const oldest = finished.keys().next().value;
        if (oldest !== undefined) finished.delete(oldest);
      }
    },
    /** Resolves when job `id` finishes (at once if it already has). */
    wait(id: JobId): Promise<JobOutcome> {
      const done = finished.get(id);
      if (done) {
        finished.delete(id);
        return Promise.resolve(done);
      }
      return new Promise((resolve) => waiters.set(id, [...(waiters.get(id) ?? []), resolve]));
    },
  };
}

export const jobResults = createJobResults();

export const waitForJob = (id: JobId) => jobResults.wait(id);
