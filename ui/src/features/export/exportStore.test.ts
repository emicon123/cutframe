import { afterEach, describe, expect, it, vi } from "vitest";

import { type ExportReport, type JobFinished, type JobProgress, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useToastStore } from "@/stores/toastStore";

import {
  cancelExport,
  type ExportState,
  exportEnd,
  exportReducer,
  finishExport,
  initialExportState,
  stopExport,
  useExportStore,
} from "./exportStore";

const progress = (jobId: string, stage: JobProgress["stage"], value: number | null): JobProgress => ({
  jobId,
  stage,
  progress: value,
});

const finished = (jobId: string, err?: "cancelled" | "io"): JobFinished => ({
  jobId,
  result: err ? { Err: { kind: err, message: "m" } } : { Ok: null },
});

const done: ExportReport = {
  jobId: "x",
  outcome: { kind: "done", path: "D:\\V\\Trip.mp4", bytes: "1048576", encoder: "nvenc", elapsedMs: 1 },
};

const reduce = (...actions: Parameters<typeof exportReducer>[1][]): ExportState =>
  actions.reduce(exportReducer, initialExportState);

afterEach(() => {
  useExportStore.setState(initialExportState);
  vi.useRealTimers();
});

describe("exportReducer", () => {
  it("tracks the stages and samples only encoding progress", () => {
    const s = reduce(
      { type: "started", id: "x" },
      { type: "progress", event: progress("x", "exportPreparing", null), now: 0 },
      { type: "progress", event: progress("x", "exportEncoding", 0.1), now: 500 },
      { type: "progress", event: progress("x", "exportEncoding", 0.2), now: 600 },
    );
    expect(s.job).toMatchObject({ id: "x", stage: "exportEncoding", progress: 0.2, encodingSince: 500 });
    expect(s.job?.samples).toHaveLength(2);
    const finalizing = exportReducer(s, {
      type: "progress",
      event: progress("x", "exportFinalizing", null),
      now: 700,
    });
    expect(finalizing.job).toMatchObject({ stage: "exportFinalizing", progress: null, encodingSince: 500 });
    expect(finalizing.job?.samples).toHaveLength(2);
  });

  it("adopts an export whose progress arrives before startExport resolved", () => {
    const s = reduce({ type: "progress", event: progress("x", "exportPreparing", null), now: 0 });
    expect(s.job?.id).toBe("x");
    expect(exportReducer(s, { type: "started", id: "x" })).toBe(s);
  });

  it("ignores other jobs (imports, the encoder probe)", () => {
    const s = reduce(
      { type: "progress", event: progress("i", "probing", 0.5), now: 0 },
      { type: "progress", event: progress("p", "probingEncoders", null), now: 0 },
    );
    expect(s.job).toBeNull();
    expect(exportEnd(s, finished("p"))).toBeNull();
  });

  it("does not bring a finished export back", () => {
    const s = reduce({ type: "started", id: "x" }, { type: "finished", id: "x" });
    expect(
      reduce({ type: "started", id: "x" }, { type: "finished", id: "x" }, { type: "started", id: "x" }).job,
    ).toBeNull();
    expect(
      exportReducer(s, { type: "progress", event: progress("x", "exportEncoding", 1), now: 0 }).job,
    ).toBeNull();
  });

  it("never lets a late first answer undo the probe result", () => {
    const probed = { probed: true, encoders: [] };
    const s = reduce({ type: "capabilities", capabilities: probed });
    expect(exportReducer(s, { type: "capabilities", capabilities: { probed: false, encoders: [] } })).toBe(s);
  });
});

describe("exportEnd", () => {
  it("uses the report, also when the job id was never seen", () => {
    const s = reduce({ type: "report", event: done });
    expect(exportEnd(s, finished("x"))).toEqual({ kind: "report", outcome: done.outcome });
  });

  it("is cancelled without a report, and an error from jobFinished otherwise", () => {
    const s = reduce({ type: "started", id: "x" });
    expect(exportEnd(s, finished("x", "cancelled"))).toEqual({ kind: "cancelled" });
    expect(exportEnd(s, finished("x", "io"))).toMatchObject({ kind: "error", error: { kind: "io" } });
  });
});

describe("finishExport", () => {
  it("clears the card and shows the done toast", () => {
    useExportStore.setState(reduce({ type: "started", id: "x" }, { type: "report", event: done }));
    finishExport(finished("x"));
    expect(useExportStore.getState().job).toBeNull();
    expect(useToastStore.getState().toasts).toMatchObject([
      { tone: "success", text: t.export.doneTitle("Trip.mp4"), detail: "1.0 MB" },
    ]);
  });

  it("shows nothing for a cancel or another job", () => {
    useExportStore.setState(reduce({ type: "started", id: "x" }));
    finishExport(finished("other"));
    expect(useExportStore.getState().job?.id).toBe("x");
    finishExport(finished("x", "cancelled"));
    expect(useExportStore.getState().job).toBeNull();
    expect(useToastStore.getState().toasts).toEqual([]);
  });
});

describe("cancel", () => {
  it("asks Rust once and marks the card as cancelling", async () => {
    const ids: string[] = [];
    mockCommands({
      cancelJob: ({ id }) => {
        ids.push(id);
        return null;
      },
    });
    useExportStore.setState(reduce({ type: "started", id: "x" }));
    await cancelExport();
    await cancelExport();
    expect(ids).toEqual(["x"]);
    expect(useExportStore.getState().job?.cancelling).toBe(true);
  });

  it("stopExport resolves when jobFinished arrives", async () => {
    mockCommands({
      cancelJob: () => {
        setTimeout(() => finishExport(finished("x", "cancelled")), 10);
        return null;
      },
    });
    useExportStore.setState(reduce({ type: "started", id: "x" }));
    await stopExport();
    expect(useExportStore.getState().job).toBeNull();
  });

  it("stopExport gives up after the timeout", async () => {
    vi.useFakeTimers();
    mockCommands({ cancelJob: () => null });
    useExportStore.setState(reduce({ type: "started", id: "x" }));
    const stopped = stopExport(2000);
    await vi.advanceTimersByTimeAsync(2000);
    await expect(stopped).resolves.toBeUndefined();
    expect(useExportStore.getState().job?.id).toBe("x");
  });

  it("stopExport does nothing without an export", async () => {
    mockCommands({});
    await expect(stopExport()).resolves.toBeUndefined();
  });
});
