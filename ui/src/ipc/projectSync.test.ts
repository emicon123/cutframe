import { afterEach, describe, expect, it, vi } from "vitest";

import { events, mockCommands, type MockCommandHandlers } from "@/bindings";
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
    tracks: [],
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
  useAppStore.setState({ info: null, lastError: null });
});

function mockStartup(
  cancelGesture: MockCommandHandlers["cancelGesture"],
  calls: string[],
  extra: Partial<MockCommandHandlers> = {},
) {
  mockCommands({
    listRecovery: () => [],
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
});
