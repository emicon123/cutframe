import { describe, expect, it } from "vitest";

import type { ExportErrorKind, ExportOutcome } from "@/bindings";
import { t } from "@/i18n";

import { exportEndToast } from "./outcome";

const mediaName = (id: string) => (id === "v" ? "beach.mp4" : "?");

const failed = (error: ExportErrorKind, extra: Partial<Extract<ExportOutcome, { kind: "failed" }>> = {}) =>
  exportEndToast(
    {
      kind: "report",
      outcome: {
        kind: "failed",
        error,
        media: null,
        keptPath: null,
        detail: "encoder said EAGAIN",
        ...extra,
      },
    },
    mediaName,
  );

describe("exportEndToast", () => {
  it("names the file and its size when done", () => {
    const toast = exportEndToast(
      {
        kind: "report",
        outcome: {
          kind: "done",
          path: "D:\\Videos\\Trip.mp4",
          bytes: "47448064",
          encoder: "nvenc",
          elapsedMs: 9000,
        },
      },
      mediaName,
    );
    expect(toast).toEqual({ tone: "success", text: t.export.doneTitle("Trip.mp4"), detail: "45.2 MB" });
  });

  it("shows nothing for a cancelled export", () => {
    expect(exportEndToast({ kind: "cancelled" }, mediaName)).toBeNull();
  });

  it("words every error kind, never the English detail", () => {
    const kinds: ExportErrorKind[] = [
      "mediaOffline",
      "decodeFailed",
      "noEncoder",
      "encoderFailed",
      "diskFull",
      "writeFailed",
      "gpu",
      "internal",
    ];
    for (const kind of kinds) {
      const toast = failed(kind);
      expect(toast).toMatchObject({ tone: "error", text: t.export.failedTitle });
      expect(toast?.detail).toBeTruthy();
      expect(toast?.detail).not.toContain("EAGAIN");
    }
    expect(failed("diskFull")?.detail).toBe(t.export.failures.diskFull);
    expect(failed("noEncoder")?.detail).toBe(t.export.failures.noEncoder);
    expect(failed("gpu")?.detail).toBe(t.export.failures.gpu);
  });

  it("names the media of mediaOffline and decodeFailed", () => {
    expect(failed("mediaOffline", { media: "v" })?.detail).toBe(t.export.failures.mediaOffline("beach.mp4"));
    expect(failed("decodeFailed", { media: "v" })?.detail).toBe(t.export.failures.decodeFailed("beach.mp4"));
    expect(failed("mediaOffline")?.detail).toBe(t.export.failures.mediaOfflineUnknown);
  });

  it("says where a finished file was kept when the rename failed", () => {
    expect(failed("writeFailed", { keptPath: "D:\\Trip.cutframe-partial.mp4" })?.detail).toBe(
      t.export.failures.writeFailedKept("D:\\Trip.cutframe-partial.mp4"),
    );
    expect(failed("writeFailed")?.detail).toBe(t.export.failures.writeFailed);
  });

  it("offers the log only where it helps", () => {
    expect(failed("mediaOffline", { media: "v" })?.action).toBeUndefined();
    expect(failed("diskFull")?.action).toBeUndefined();
    expect(failed("encoderFailed")?.action).toBe("openLogs");
    expect(failed("internal")?.action).toBe("openLogs");
  });

  it("words a failure without a report from its IpcError", () => {
    const toast = exportEndToast({ kind: "error", error: { kind: "io", message: "denied" } }, mediaName);
    expect(toast).toMatchObject({ tone: "error", text: t.export.failedTitle, detail: t.errors.kinds.io });
  });
});
