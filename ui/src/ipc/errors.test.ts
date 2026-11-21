import { afterEach, describe, expect, it, vi } from "vitest";

import type { IpcError } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

import { describeError, logError } from "./errors";

const invalidEdit: IpcError = { kind: "invalidEdit", message: "unknown track 42" };

afterEach(() => {
  vi.restoreAllMocks();
  useAppStore.setState({ lastError: null });
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

describe("appStore.reportError", () => {
  it("stores the translated text and logs the message", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    useAppStore.getState().reportError(invalidEdit);
    expect(useAppStore.getState().lastError).toBe(t.errors.kinds.invalidEdit);
    expect(spy).toHaveBeenCalledOnce();
  });
});
