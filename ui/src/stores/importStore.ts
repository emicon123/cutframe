import { create } from "zustand";

import type {
  IpcError,
  JobFinished,
  JobId,
  JobProgress_Deserialize,
  JobStage,
  MediaErrorKind,
  MediaFileResult,
  MediaJobReport,
} from "@/bindings";

/**
 * UI state of running import jobs and the result of the last one. This is display state only:
 * what was imported lives in the project model (Rust) and arrives through `project-changed`.
 */

/**
 * A running job as the media panel shows it: an import the UI started, or a background job Rust
 * started by itself (preview generation, see `isBackgroundJob`), which arrives through the same
 * events.
 */
export type ImportJob = {
  id: JobId;
  /** `null` until the first `job-progress` event. */
  stage: JobStage | null;
  /** 0..=1, or `null` while the amount of work is not known yet. */
  progress: number | null;
  /** The user asked to cancel; waiting for `job-finished`. */
  cancelling: boolean;
  /** Per-file results, from `media-job-report` (just before `job-finished`). */
  files: MediaFileResult[] | null;
};

/** Stages of jobs Rust starts by itself; they get no Cancel button and no result toast. */
const BACKGROUND_STAGES: readonly JobStage[] = ["generatingPreviews"];

export function isBackgroundJob(job: Pick<ImportJob, "stage">): boolean {
  return job.stage !== null && BACKGROUND_STAGES.includes(job.stage);
}

export type FailedFile = { path: string; error: MediaErrorKind };

/** What the result toast shows for a finished import. */
export type ImportSummary = {
  jobId: JobId;
  added: number;
  alreadyImported: number;
  failed: FailedFile[];
  /** Set when the whole job failed (not cancelled); `failed` etc. are then empty. */
  jobError: IpcError["kind"] | null;
};

export type ImportState = {
  /** In start order. */
  jobs: ImportJob[];
  /** Recently finished ids, so a late `started` or `progress` does not bring a job back. */
  finished: JobId[];
  summary: ImportSummary | null;
};

export type ImportAction =
  | { type: "started"; id: JobId }
  | { type: "progress"; event: JobProgress_Deserialize }
  | { type: "report"; event: MediaJobReport }
  | { type: "finished"; event: JobFinished }
  | { type: "cancelRequested"; id: JobId }
  | { type: "dismissSummary" };

/** How many finished ids are remembered; events of one job arrive within moments of each other. */
const FINISHED_MEMORY = 32;

export const initialImportState: ImportState = { jobs: [], finished: [], summary: null };

/** Counts the per-file outcomes of a media job report, keeping failures in input order. */
export function summarizeReport(jobId: JobId, files: readonly MediaFileResult[]): ImportSummary {
  const summary: ImportSummary = { jobId, added: 0, alreadyImported: 0, failed: [], jobError: null };
  for (const { path, outcome } of files) {
    switch (outcome.kind) {
      case "added":
        summary.added += 1;
        break;
      case "alreadyImported":
        summary.alreadyImported += 1;
        break;
      case "failed":
        summary.failed.push({ path, error: outcome.error });
        break;
    }
  }
  return summary;
}

/** True when the summary has nothing the user must read (every file was added). */
export function isCleanSummary(summary: ImportSummary): boolean {
  return summary.jobError === null && summary.failed.length === 0 && summary.alreadyImported === 0;
}

function newJob(id: JobId): ImportJob {
  return { id, stage: null, progress: null, cancelling: false, files: null };
}

/**
 * Applies one job event or user action. Events can arrive in any order relative to the command
 * that started the job (a small import may finish before `importMedia` resolves), so every event
 * creates the job if it is missing, and anything for a finished id is ignored.
 */
export function importReducer(state: ImportState, action: ImportAction): ImportState {
  const update = (id: JobId, change: (job: ImportJob) => ImportJob): ImportState => {
    if (state.finished.includes(id)) return state;
    const exists = state.jobs.some((job) => job.id === id);
    const jobs = exists
      ? state.jobs.map((job) => (job.id === id ? change(job) : job))
      : [...state.jobs, change(newJob(id))];
    return { ...state, jobs };
  };

  switch (action.type) {
    case "started":
      return update(action.id, (job) => job);
    case "progress": {
      const { jobId, stage, progress } = action.event;
      return update(jobId, (job) => ({ ...job, stage, progress: progress ?? null }));
    }
    case "report":
      return update(action.event.jobId, (job) => ({ ...job, files: action.event.files }));
    case "cancelRequested":
      return state.jobs.some((job) => job.id === action.id)
        ? update(action.id, (job) => ({ ...job, cancelling: true }))
        : state;
    case "finished": {
      const { jobId, result } = action.event;
      if (state.finished.includes(jobId)) return state;
      const job = state.jobs.find((j) => j.id === jobId);
      const finished = [...state.finished, jobId].slice(-FINISHED_MEMORY);
      const jobs = state.jobs.filter((j) => j.id !== jobId);
      let summary = state.summary;
      if (job && isBackgroundJob(job)) {
        // Background work reports nothing: failed previews show placeholders instead.
      } else if (result.Err) {
        if (result.Err.kind !== "cancelled") {
          summary = { jobId, added: 0, alreadyImported: 0, failed: [], jobError: result.Err.kind };
        }
      } else if (job?.files) {
        summary = summarizeReport(jobId, job.files);
      }
      return { jobs, finished, summary };
    }
    case "dismissSummary":
      return { ...state, summary: null };
  }
}

type ImportStore = ImportState & { dispatch: (action: ImportAction) => void };

export const useImportStore = create<ImportStore>()((set) => ({
  ...initialImportState,
  dispatch: (action) => set((state) => importReducer(state, action)),
}));
