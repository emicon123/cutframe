import { create } from "zustand";

import {
  commands,
  type ExportCapabilities,
  type ExportReport,
  type JobFinished,
  type JobId,
  type JobProgress,
  type JobStage,
  type MediaId,
} from "@/bindings";
import { t } from "@/i18n";
import { run } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";
import { showToast } from "@/stores/toastStore";

import { addSample, type ProgressSample } from "./eta";
import { type ExportEnd, exportEndToast } from "./outcome";

/** The running export as the progress card shows it (export.md §7). */
export type ExportJob = {
  id: JobId;
  /** `null` until the first `jobProgress`. */
  stage: JobStage | null;
  /** 0..=1 while encoding; `null` in the stages without a value. */
  progress: number | null;
  /** Encoding progress of the last 5 s, for the ETA. */
  samples: ProgressSample[];
  /** When the first encoding progress arrived (same clock as `samples`). */
  encodingSince: number | null;
  /** Cancel was asked; waiting for `jobFinished`. */
  cancelling: boolean;
};

export type ExportState = {
  dialogOpen: boolean;
  /** The session's encoder probe; `null` until the dialog first asked. */
  capabilities: ExportCapabilities | null;
  /** One export at a time; `null` when none runs. */
  job: ExportJob | null;
  /** The report of an export whose `jobFinished` has not arrived yet. */
  report: ExportReport | null;
  /** Recently finished ids, so a late `started` or progress does not bring a job back. */
  finished: JobId[];
};

export type ExportAction =
  | { type: "started"; id: JobId }
  | { type: "progress"; event: JobProgress; now: number }
  | { type: "report"; event: ExportReport }
  | { type: "finished"; id: JobId }
  | { type: "cancelRequested" }
  | { type: "capabilities"; capabilities: ExportCapabilities }
  | { type: "dialog"; open: boolean };

const EXPORT_STAGES: ReadonlySet<JobStage> = new Set<JobStage>([
  "exportPreparing",
  "exportEncoding",
  "exportFinalizing",
]);

const FINISHED_MEMORY = 8;

export const initialExportState: ExportState = {
  dialogOpen: false,
  capabilities: null,
  job: null,
  report: null,
  finished: [],
};

const newJob = (id: JobId): ExportJob => ({
  id,
  stage: null,
  progress: null,
  samples: [],
  encodingSince: null,
  cancelling: false,
});

/**
 * Applies one event or user action. Events can arrive before `startExport` resolves, and only one
 * export runs at a time, so progress with an export stage is the running export whether or not its
 * id is known yet (this also picks up an export a reloaded webview did not start).
 */
export function exportReducer(state: ExportState, action: ExportAction): ExportState {
  switch (action.type) {
    case "started":
      if (state.finished.includes(action.id) || state.job?.id === action.id) return state;
      return { ...state, job: newJob(action.id) };
    case "progress": {
      const { jobId, stage, progress = null } = action.event;
      if (!EXPORT_STAGES.has(stage) || state.finished.includes(jobId)) return state;
      const job = state.job?.id === jobId ? state.job : newJob(jobId);
      const encoding = stage === "exportEncoding" && progress !== null;
      return {
        ...state,
        job: {
          ...job,
          stage,
          progress,
          samples: encoding ? addSample(job.samples, { at: action.now, progress }) : job.samples,
          encodingSince: encoding ? (job.encodingSince ?? action.now) : job.encodingSince,
        },
      };
    }
    case "report":
      return { ...state, report: action.event };
    case "finished": {
      if (state.job?.id !== action.id && state.report?.jobId !== action.id) return state;
      return {
        ...state,
        job: null,
        report: null,
        finished: [...state.finished, action.id].slice(-FINISHED_MEMORY),
      };
    }
    case "cancelRequested":
      return state.job ? { ...state, job: { ...state.job, cancelling: true } } : state;
    case "capabilities":
      // The probe result never goes back to "not probed" (a late first answer after the event).
      if (state.capabilities?.probed && !action.capabilities.probed) return state;
      return { ...state, capabilities: action.capabilities };
    case "dialog":
      return { ...state, dialogOpen: action.open };
  }
}

/**
 * How the export with this `jobFinished` ended, or `null` when it is not the export (any other
 * job). A cancelled export has no report; a failure without one is worded from the `IpcError`.
 */
export function exportEnd(state: ExportState, event: JobFinished): ExportEnd | null {
  const report = state.report?.jobId === event.jobId ? state.report : null;
  if (state.job?.id !== event.jobId && report === null) return null;
  if (report) return { kind: "report", outcome: report.outcome };
  const error = event.result.Err;
  if (!error || error.kind === "cancelled") return { kind: "cancelled" };
  return { kind: "error", error };
}

type ExportStore = ExportState & { dispatch: (action: ExportAction) => void };

export const useExportStore = create<ExportStore>()((set) => ({
  ...initialExportState,
  dispatch: (action) => set((state) => exportReducer(state, action)),
}));

const dispatch = (action: ExportAction) => useExportStore.getState().dispatch(action);

/** The file name of a media id of the open project, for issue and failure texts. */
export function mediaName(id: MediaId): string {
  const media = useProjectStore.getState().state?.project.media.find((m) => m.id === id);
  return media?.fileName ?? t.export.unknownMedia;
}

/** Ends the export of `event` (if it is one): clears the card and shows the toast. */
export function finishExport(event: JobFinished): void {
  const end = exportEnd(useExportStore.getState(), event);
  if (end === null) return;
  dispatch({ type: "finished", id: event.jobId });
  if (end.kind === "report" && end.outcome.kind === "failed") {
    // English diagnostics for developers; the toast says it in words (ADR-0003 §5).
    console.error(`[export] ${end.outcome.error}: ${end.outcome.detail}`);
  }
  const toast = exportEndToast(end, mediaName);
  if (toast) showToast(toast);
}

export function openExportDialog(): void {
  dispatch({ type: "dialog", open: true });
}

export function closeExportDialog(): void {
  dispatch({ type: "dialog", open: false });
}

export function exportStarted(id: JobId): void {
  dispatch({ type: "started", id });
}

export function setCapabilities(capabilities: ExportCapabilities): void {
  dispatch({ type: "capabilities", capabilities });
}

export function exportRunning(): boolean {
  return useExportStore.getState().job !== null;
}

/** The progress card's Cancel: asks Rust to stop; the card stays until `jobFinished`. */
export async function cancelExport(): Promise<void> {
  const job = useExportStore.getState().job;
  if (!job || job.cancelling) return;
  dispatch({ type: "cancelRequested" });
  await run(commands.cancelJob(job.id));
}

/** How long quitting waits for a cancelled export to clean up (export.md §5.1). */
export const QUIT_WAIT_MS = 2000;

/**
 * Cancels the running export and resolves once its `jobFinished` arrived, or after `timeoutMs`
 * (then `confirmClose` cancels it again). Resolves at once when no export runs.
 */
export async function stopExport(timeoutMs = QUIT_WAIT_MS): Promise<void> {
  if (!exportRunning()) return;
  const ended = new Promise<void>((resolve) => {
    const timer = setTimeout(done, timeoutMs);
    const unsubscribe = useExportStore.subscribe((s) => {
      if (s.job === null) done();
    });
    function done() {
      clearTimeout(timer);
      unsubscribe();
      resolve();
    }
  });
  await cancelExport();
  if (exportRunning()) await ended;
}
