import { afterEach, describe, expect, it, vi } from "vitest";

import { events, mockCommands } from "@/bindings";
import { initialExportState, useExportStore } from "@/features/export/exportStore";
import { projectState } from "@/features/inspector/testProject";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";
import { useToastStore } from "@/stores/toastStore";

import { startExportSync } from "./exportSync";

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  useExportStore.setState(initialExportState);
  useProjectStore.setState({ state: null });
  vi.restoreAllMocks();
});

describe("startExportSync", () => {
  it("follows an export from progress to the failure toast with the media name", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockCommands({});
    useProjectStore.setState({ state: projectState([]) });
    stop = startExportSync();
    await vi.waitFor(async () => {
      await events.jobProgress.emit({ jobId: "x", stage: "exportEncoding", progress: 0.5 });
      expect(useExportStore.getState().job?.progress).toBe(0.5);
    });

    await events.exportReport.emit({
      jobId: "x",
      outcome: { kind: "failed", error: "mediaOffline", media: "v", keptPath: null, detail: "gone" },
    });
    await events.jobFinished.emit({ jobId: "x", result: { Err: { kind: "internal", message: "failed" } } });
    await vi.waitFor(() => expect(useExportStore.getState().job).toBeNull());
    expect(useToastStore.getState().toasts).toMatchObject([
      { tone: "error", text: t.export.failedTitle, detail: t.export.failures.mediaOffline("v.file") },
    ]);
  });

  it("stores the encoder probe result", async () => {
    mockCommands({});
    stop = startExportSync();
    const caps = {
      probed: true,
      encoders: [{ codec: "h264" as const, encoder: "openH264" as const, hardware: false, available: true }],
    };
    await vi.waitFor(async () => {
      await events.exportCapabilitiesChanged.emit(caps);
      expect(useExportStore.getState().capabilities).toEqual(caps);
    });
  });
});
