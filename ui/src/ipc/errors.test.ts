import { afterEach, describe, expect, it, vi } from "vitest";

import type { IpcError } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { useToastStore } from "@/stores/toastStore";

import { describeError, errorToast, hasLoggedDetails, logError } from "./errors";

const invalidEdit: IpcError = { kind: "invalidEdit", message: "unknown track 42" };
const io: IpcError = { kind: "io", message: "os error 32: file is locked" };

afterEach(() => {
  vi.restoreAllMocks();
});

describe("describeError", () => {
  it("translates the kind and never shows the diagnostic message", () => {
    const text = describeError(invalidEdit);
    expect(text).toBe(t.errors.kinds.invalidEdit);
    expect(text).not.toContain(invalidEdit.message);
  });

  it("uses generic text for an unknown kind", () => {
    expect(describeError({ kind: "diskFull", message: "no space left" })).toBe(t.errors.unknown);
  });

  it("uses generic text for failures that are not IpcErrors", () => {
    expect(describeError(new Error("listen failed"))).toBe(t.errors.unknown);
    expect(describeError("command not allowed")).toBe(t.errors.unknown);
    expect(describeError(undefined)).toBe(t.errors.unknown);
  });
});

describe("logError", () => {
  it("logs the diagnostic message to the console", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    logError(invalidEdit);
    expect(spy).toHaveBeenCalledWith("[ipc] invalidEdit: unknown track 42");
  });
});

describe("hasLoggedDetails", () => {
  it("is true for backend failures and false for self-explaining kinds and JS errors", () => {
    expect(hasLoggedDetails(io)).toBe(true);
    expect(hasLoggedDetails({ kind: "internal", message: "panic" })).toBe(true);
    // A kind this UI build does not know is still logged by Rust.
    expect(hasLoggedDetails({ kind: "diskFull", message: "" })).toBe(true);
    expect(hasLoggedDetails(invalidEdit)).toBe(false);
    expect(hasLoggedDetails({ kind: "overlap", message: "" })).toBe(false);
    expect(hasLoggedDetails(new Error("listen failed"))).toBe(false);
  });
});

describe("errorToast", () => {
  it("shows the translated reason with an Open logs button when the log has details", () => {
    expect(errorToast(io)).toEqual({
      tone: "error",
      text: t.errors.kinds.io,
      detail: t.toasts.detailsInLog,
      action: "openLogs",
    });
  });

  it("puts the reason under a title", () => {
    expect(errorToast(io, "Title")).toEqual({
      tone: "error",
      text: "Title",
      detail: t.errors.kinds.io,
      action: "openLogs",
    });
  });

  it("has no log hint for a rejected edit or a failure Rust did not log", () => {
    expect(errorToast(invalidEdit)).toEqual({ tone: "error", text: t.errors.kinds.invalidEdit });
    expect(errorToast("command not allowed")).toEqual({ tone: "error", text: t.errors.unknown });
  });

  it("never contains the diagnostic message", () => {
    expect(JSON.stringify(errorToast(io))).not.toContain(io.message);
  });
});

describe("appStore.reportError", () => {
  it("shows an error toast with the translated text and logs the message", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    useAppStore.getState().reportError(invalidEdit);
    expect(useToastStore.getState().toasts.at(-1)).toMatchObject({
      tone: "error",
      text: t.errors.kinds.invalidEdit,
    });
    expect(spy).toHaveBeenCalledOnce();
  });
});
