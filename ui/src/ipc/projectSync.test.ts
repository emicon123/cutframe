import { afterEach, describe, expect, it, vi } from "vitest";

import { events, mockCommands, type MockCommandHandlers, type PlaybackTick } from "@/bindings";
import { answerCancelExport, useQuitPrompt } from "@/features/export/quitPrompt";
import { fileActions } from "@/features/project/fileActions";
import { recovery } from "@/features/project/recovery";
import { playheadStore } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useAppStore } from "@/stores/appStore";
import { useToastStore } from "@/stores/toastStore";
import { useUiStore } from "@/stores/uiStore";

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

const paused: PlaybackTick = {
  time: 0,
  state: "paused",
  rate: 0,
  looping: false,
  seek: 0,
  droppedFrames: 0,
  target: { kind: "timeline" },
};

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  vi.restoreAllMocks();
  useProjectStore.setState({ state: null });
  useAppStore.setState({ info: null, gpuNotice: null });
  playheadStore.setState({ time: 0, state: "paused", rate: 0, looping: false, seekGeneration: null });
  useUiStore.setState({ view: "home", started: false });
});

function mockStartup(
  cancelGesture: MockCommandHandlers["cancelGesture"],
  calls: string[],
  extra: Partial<MockCommandHandlers> = {},
) {
  mockCommands({
    listRecovery: () => [],
    takeLaunchPath: () => null,
    seek: () => 1,
    getPlayback: () => paused,
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
    expect(useToastStore.getState().toasts).toEqual([]);
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
    await events.quitRequested.emit({ unsaved: false, exporting: false });
    await vi.waitFor(() => expect(calls).toContain("confirmClose"));
  });

  it("asks before closing while an export runs, even one this webview did not start", async () => {
    const calls: string[] = [];
    mockStartup(() => null, calls, {
      confirmClose: () => {
        calls.push("confirmClose");
        return null;
      },
    });

    stop = startProjectSync();
    await vi.waitFor(() => expect(useProjectStore.getState().state?.revision).toBe(3));
    await events.quitRequested.emit({ unsaved: false, exporting: true });
    await vi.waitFor(() => expect(useQuitPrompt.getState().resolve).not.toBeNull());
    expect(calls).not.toContain("confirmClose");
    answerCancelExport(true);
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

  it("a launch path opens the editor directly, without showing Home first", async () => {
    const views: string[] = [];
    mockStartup(() => null, [], {
      takeLaunchPath: () => "D:\\v\\a.cutframe",
      openProject: () => {
        const { view, started } = useUiStore.getState();
        views.push(`${view} started=${started}`);
        return null;
      },
    });
    vi.spyOn(recovery, "load").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(useUiStore.getState().started).toBe(true));
    expect(views).toEqual(["home started=false"]);
    expect(useUiStore.getState().view).toBe("editor");
  });

  it("without a launch path the home screen is shown", async () => {
    mockStartup(() => null, []);
    const load = vi.spyOn(recovery, "load").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    expect(useUiStore.getState()).toMatchObject({ view: "home", started: true });
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

  it("mirrors the transport: getPlayback once, then every playbackTick", async () => {
    let fetched = 0;
    mockStartup(() => null, [], {
      getPlayback: () => {
        fetched += 1;
        return { ...paused, time: 4200, looping: true, seek: 7 };
      },
    });
    vi.spyOn(recovery, "load").mockResolvedValue();

    stop = startProjectSync();
    await vi.waitFor(() => expect(playheadStore.getState().time).toBe(4200));
    expect(playheadStore.getState()).toMatchObject({ looping: true, seekGeneration: 7 });
    expect(fetched).toBe(1);

    await events.playbackTick.emit({ ...paused, time: 9000, state: "playing", rate: 1, seek: 7 });
    await vi.waitFor(() => expect(playheadStore.getState()).toMatchObject({ time: 9000, state: "playing" }));
    // Emitted before a newer seek the UI already knows about: ignored.
    await events.playbackTick.emit({ ...paused, time: 10, seek: 6 });
    await Promise.resolve();
    expect(playheadStore.getState().time).toBe(9000);

    stop();
    stop = undefined;
    await events.playbackTick.emit({ ...paused, time: 1, seek: 8 });
    await Promise.resolve();
    expect(playheadStore.getState().time).toBe(9000);
  });
});
