import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { mockCommands } from "@/bindings";
import { t } from "@/i18n";

import { type ExportJob, initialExportState, useExportStore } from "./exportStore";
import { ExportProgress } from "./ExportProgress";
import { QuitExportDialog } from "./QuitExportDialog";
import { askCancelExport } from "./quitPrompt";

const job = (overrides: Partial<ExportJob> = {}): ExportJob => ({
  id: "x",
  stage: "exportEncoding",
  progress: 0.42,
  samples: [],
  encodingSince: null,
  cancelling: false,
  ...overrides,
});

afterEach(() => {
  useExportStore.setState(initialExportState);
});

describe("ExportProgress", () => {
  it("is hidden without an export", () => {
    const { container } = render(<ExportProgress />);
    expect(container.textContent).toBe("");
  });

  it("shows the stage, percent and an ETA while encoding", () => {
    const now = performance.now();
    // 1 % per second over the last 4 s, encoding for 10 s: about 58 s left.
    const samples = [0, 1, 2, 3, 4].map((i) => ({ at: now - 4000 + i * 1000, progress: 0.38 + i * 0.01 }));
    useExportStore.setState({ job: job({ samples, encodingSince: now - 10_000 }) });
    render(<ExportProgress />);
    expect(screen.getByText(t.mediaBin.job.stages.exportEncoding)).toBeTruthy();
    expect(screen.getByText(t.export.progress.percent(42))).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("42");
    expect(screen.getByText(t.export.progress.etaMinutes(1))).toBeTruthy();
  });

  it("shows no percent and no ETA in the stages without progress", () => {
    useExportStore.setState({ job: job({ stage: "exportPreparing", progress: null }) });
    render(<ExportProgress />);
    expect(screen.getByText(t.mediaBin.job.stages.exportPreparing)).toBeTruthy();
    expect(screen.queryByText(/%/)).toBeNull();
    expect(screen.queryByText(t.export.progress.estimating)).toBeNull();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBeNull();
  });

  it("estimates nothing in the first seconds", () => {
    useExportStore.setState({ job: job({ encodingSince: performance.now() }) });
    render(<ExportProgress />);
    expect(screen.getByText(t.export.progress.estimating)).toBeTruthy();
  });

  it("cancels the job and waits for jobFinished", async () => {
    const cancelled: string[] = [];
    mockCommands({
      cancelJob: ({ id }) => {
        cancelled.push(id);
        return null;
      },
    });
    useExportStore.setState({ job: job() });
    render(<ExportProgress />);
    fireEvent.click(screen.getByRole("button", { name: t.export.progress.cancel }));
    await waitFor(() => expect(cancelled).toEqual(["x"]));
    expect(screen.getByRole("button", { name: t.export.progress.cancelling })).toHaveProperty(
      "disabled",
      true,
    );
  });
});

describe("QuitExportDialog", () => {
  it("answers true for Cancel export and quit, false for Keep exporting", async () => {
    render(<QuitExportDialog />);
    const quit = askCancelExport();
    fireEvent.click(await screen.findByRole("button", { name: t.export.quit.confirm }));
    await expect(quit).resolves.toBe(true);

    const stay = askCancelExport();
    fireEvent.click(await screen.findByRole("button", { name: t.export.quit.keep }));
    await expect(stay).resolves.toBe(false);
  });
});
