import { afterEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type MockCommandHandlers } from "@/bindings";
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
  history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
};

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  vi.restoreAllMocks();
  useProjectStore.setState({ state: null });
  useAppStore.setState({ info: null, lastError: null });
});

function mockStartup(cancelGesture: MockCommandHandlers["cancelGesture"], calls: string[]) {
  mockCommands({
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
});
