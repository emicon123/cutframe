import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type IpcError, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { showToast, TOAST_DURATION_MS, useToastStore } from "@/stores/toastStore";

import { closeErrorDialog, errorDialogContent, showErrorDialog } from "./errorDialog";
import { ErrorDialog } from "./ErrorDialog";
import { openLogsFailedToast, openLogsFolder } from "./logs";
import { Toaster } from "./Toaster";

const io: IpcError = { kind: "io", message: "os error 5: access denied" };
const malformed: IpcError = { kind: "malformedProject", message: "expected value at line 1" };

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  closeErrorDialog();
  useAppStore.setState({ previewOccluders: 0 });
});

describe("errorDialogContent", () => {
  it("a failed open shows the translated reason without Save As", () => {
    expect(errorDialogContent("open", malformed)).toEqual({
      title: t.errorDialog.openTitle,
      reason: t.errors.kinds.malformedProject,
      logs: true,
      saveAs: false,
    });
  });

  it("a failed save offers Save As and the log", () => {
    expect(errorDialogContent("save", io)).toEqual({
      title: t.errorDialog.saveTitle,
      reason: t.errors.kinds.io,
      hint: t.errorDialog.saveHint,
      logs: true,
      saveAs: true,
    });
  });

  it("a failure Rust did not log has no log button", () => {
    expect(errorDialogContent("save", new Error("plugin")).logs).toBe(false);
  });
});

describe("openLogsFailedToast", () => {
  it("explains a build without a log file and never offers Open logs again", () => {
    expect(openLogsFailedToast({ kind: "unavailable", message: "no log dir" })).toEqual({
      tone: "error",
      text: t.logs.unavailable,
    });
    expect(openLogsFailedToast(io)).toEqual({
      tone: "error",
      text: t.logs.openFailed,
      detail: t.errors.kinds.io,
    });
  });
});

describe("openLogsFolder", () => {
  it("calls the command and shows a toast when it fails", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const calls: string[] = [];
    mockCommands({
      openLogsFolder: () => {
        calls.push("open");
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Tauri rejects with the IpcError
        return Promise.reject(io);
      },
    });
    await openLogsFolder();
    expect(calls).toEqual(["open"]);
    expect(useToastStore.getState().toasts.at(-1)?.text).toBe(t.logs.openFailed);
  });
});

describe("Toaster", () => {
  it("auto-dismisses info toasts and keeps errors longer", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      showToast({ tone: "info", text: "Saved" });
      showToast({ tone: "error", text: "Broken" });
    });
    expect(screen.getByRole("status").textContent).toContain("Saved");
    expect(screen.getByRole("alert").textContent).toContain("Broken");

    act(() => {
      vi.advanceTimersByTime(TOAST_DURATION_MS.info);
    });
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("alert")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(TOAST_DURATION_MS.error - TOAST_DURATION_MS.info);
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("stays while hovered", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => showToast({ tone: "info", text: "Saved" }));
    fireEvent.pointerEnter(screen.getByRole("status"));
    act(() => {
      vi.advanceTimersByTime(TOAST_DURATION_MS.info * 3);
    });
    expect(screen.getByRole("status")).toBeTruthy();
  });

  it("an error toast's Open logs button opens the logs folder; ✕ dismisses", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const calls: string[] = [];
    mockCommands({ openLogsFolder: () => (calls.push("open"), null) });
    render(<Toaster />);
    act(() => useAppStore.getState().reportError(io));

    const toast = screen.getByRole("alert");
    expect(toast.textContent).toContain(t.errors.kinds.io);
    expect(toast.textContent).not.toContain(io.message);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: t.toasts.openLogs }));
      await Promise.resolve();
    });
    expect(calls).toEqual(["open"]);

    fireEvent.click(screen.getByRole("button", { name: t.toasts.dismiss }));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("ErrorDialog", () => {
  it("shows a failed save with Save As and hides the preview while open", async () => {
    render(<ErrorDialog />);
    act(() => showErrorDialog("save", io));
    const dialog = await screen.findByRole("dialog", { name: t.errorDialog.saveTitle });
    expect(dialog.textContent).toContain(t.errors.kinds.io);
    expect(dialog.textContent).not.toContain(io.message);
    expect(screen.getByRole("button", { name: t.errorDialog.saveAs })).toBeTruthy();
    expect(screen.getByRole("button", { name: t.errorDialog.openLogs })).toBeTruthy();
    expect(useAppStore.getState().previewOccluders).toBe(1);

    fireEvent.click(screen.getByRole("button", { name: t.errorDialog.close }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().previewOccluders).toBe(0);
  });
});
