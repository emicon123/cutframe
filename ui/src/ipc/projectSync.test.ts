import { afterEach, describe, expect, it, vi } from "vitest";

import { events, mockCommands, type MockCommandHandlers } from "@/bindings";
import { fileActions } from "@/features/project/fileActions";
import { recovery } from "@/features/project/recovery";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useAppStore } from "@/stores/appStore";

import { startProjectSync } from "./projectSync";

const project: ProjectState = {
  revision: 3,
  project: {
    id: "p",
    name: "Untitled",
    settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
    duration: 0,
    background: { kind: "color", color: "#000000ff" },
    mainTrack: "main",
    magnetic: true,
    tracks: [],
    media: [],
  },
  history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null, entries: [], current: 0 },
  filePath: null,
  dirty: false,
};

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  vi.restoreAllMocks();
  useProjectStore.setState({ state: null });
  useAppStore.setState({ info: null, lastError: null, gpuNotice: null });
});

function mockStartup(
  cancelGesture: MockCommandHandlers["cancelGesture"],
  calls: string[],
  extra: Partial<MockCommandHandlers> = {},
) {
  mockCommands({
    listRecovery: () => [],
    takeLaunchPath: () => null,
    ...extra,
    cancelGesture,
    getProject: () => {
      calls.push("getProject");
      return project;
    },
    getAppInfo: () => {
      calls.push("getAppInfo");
      return { version: "0.0.0", gpu: null, previewMode: "frames" };
    },
  });
}

describe("startProjectSync", () => {
  it("cancels a stale gesture before fetching the initial state", async () => {
    const calls: string[] = [];
    mockStartup(() => {
      calls.push("cancelGesture");
      return null;
    }, calls);

    stop = startProjectSync();
    await vi.waitFor(() => expect(useProjectStore.getState().state?.revision).toBe(3));
    expect(calls[0]).toBe("cancelGesture");
    expect(calls).toHaveLength(3);
  });

  it("still loads the project when the cancel fails", async () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const calls: string[] = [];
    mockStartup(() => {
      throw new Error("cancel_gesture not allowed");
    }, calls);

    stop = startProjectSync();
    await vi.waitFor(() => expect(useProjectStore.getState().state?.revision).toBe(3));
    expect(useAppStore.getState().info?.version).toBe("0.0.0");
    expect(useAppStore.getState().lastError).toBeNull();
    expect(spy).toHaveBeenCalled();
  });

  it("closes the window on quitRequested when nothing is unsaved", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls, {
      confirmClose: () => {
        calls.push("confirmClose");
        return null;
      },
    });

    stop = startProjectSync();
    await vi.waitFor(() => expect(useProjectStore.getState().state?.revision).toBe(3));
    await events.quitRequested.emit();
    await vi.waitFor(() => expect(calls).toContain("confirmClose"));
  });

  it("offers crash recovery after the first state arrived", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls);
    const load = vi.spyOn(recovery, "load").mockImplementation(() => {
      calls.push(`load at revision ${useProjectStore.getState().state?.revision}`);
      return Promise.resolve();
    });

    stop = startProjectSync();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    expect(calls.at(-1)).toBe("load at revision 3");
  });

  it("opens the launch path before the recovery check", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls, {
      takeLaunchPath: () => {
        calls.push("takeLaunchPath");
        return "D:\\v\\a.cutframe";
      },
      openProject: ({ path }) => {
        calls.push(`openProject ${path}`);
        return null;
      },
    });
    const load = vi.spyOn(recovery, "load").mockImplementation(() => {
      calls.push("recovery");
      return Promise.resolve();
    });

    stop = startProjectSync();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    expect(calls.slice(-3)).toEqual(["takeLaunchPath", "openProject D:\\v\\a.cutframe", "recovery"]);
  });

  it("still offers recovery when there is no launch path or taking it fails", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const calls: string[] = [];
    mockStartup(() => null, calls, {
      takeLaunchPath: () => {
        throw new Error("not allowed");
      },
    });
    const openPath = vi.spyOn(fileActions, "openPath");
    const load = vi.spyOn(recovery, "load").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    expect(openPath).not.toHaveBeenCalled();
  });

  it("opens a file forwarded by a second launch through the guarded open", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls);
    vi.spyOn(recovery, "load").mockResolvedValue();
    const openPath = vi.spyOn(fileActions, "openPath").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(useProjectStore.getState().state?.revision).toBe(3));
    await events.openFileRequested.emit({ path: "E:\\b.cutframe" });
    await vi.waitFor(() => expect(openPath).toHaveBeenCalledWith("E:\\b.cutframe"));
  });

  it("replaces the app info on appInfoChanged and shows the GPU notice", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls);
    vi.spyOn(recovery, "load").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(useAppStore.getState().info?.version).toBe("0.0.0"));
    await events.appInfoChanged.emit({ version: "0.0.0", gpu: null, previewMode: "unavailable" });
    await vi.waitFor(() => expect(useAppStore.getState().info?.previewMode).toBe("unavailable"));
    expect(useAppStore.getState().gpuNotice).toEqual({ kind: "previewLost" });
  });
});
