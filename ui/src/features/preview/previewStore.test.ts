import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type PreviewQuality, type PreviewView_Deserialize } from "@/bindings";
import type { ProjectState } from "@/stores/projectStore";

const setFullscreen = vi.fn<(on: boolean) => Promise<void>>();
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFullscreen }) }));

type PreviewStoreModule = typeof import("./previewStore");

let store: PreviewStoreModule;
let sent: string[];

// A manual animation-frame clock for the pan drag.
let frames: (() => void)[];
const nextFrame = () => {
  const due = frames;
  frames = [];
  for (const callback of due) callback();
};

const viewText = (view: PreviewView_Deserialize) =>
  view.zoom.kind === "fit"
    ? `view fit ${view.panX} ${view.panY}`
    : `view ${view.zoom.percent} ${view.panX?.toFixed(3)} ${view.panY?.toFixed(3)}`;

function mockPreview() {
  mockCommands({
    setPreviewQuality: ({ quality }: { quality: PreviewQuality }) => {
      sent.push(`quality ${quality}`);
    },
    setPreviewView: ({ view }) => {
      sent.push(viewText(view));
    },
  });
}

/** A fresh module, so the stored quality is read again as at app start. */
async function load() {
  vi.resetModules();
  store = await import("./previewStore");
  const { useProjectStore } = await import("@/stores/projectStore");
  useProjectStore.setState({
    state: { project: { settings: { width: 1920, height: 1080 } } } as unknown as ProjectState,
  });
}

beforeEach(async () => {
  sent = [];
  frames = [];
  vi.stubGlobal("requestAnimationFrame", (callback: () => void) => frames.push(callback));
  window.localStorage.clear();
  setFullscreen.mockReset();
  setFullscreen.mockResolvedValue(undefined);
  mockPreview();
  await load();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("preview quality", () => {
  it("is full by default and sent with the view at startup", async () => {
    expect(store.usePreviewStore.getState().quality).toBe("full");
    store.syncPreviewSettings();
    await vi.waitFor(() => expect(sent).toEqual(["quality full", "view fit 0 0"]));
  });

  it("is remembered across restarts and sent on change", async () => {
    store.setPreviewQuality("quarter");
    expect(store.usePreviewStore.getState().quality).toBe("quarter");
    await vi.waitFor(() => expect(sent).toEqual(["quality quarter"]));

    await load();
    expect(store.usePreviewStore.getState().quality).toBe("quarter");
    sent = [];
    store.syncPreviewSettings();
    await vi.waitFor(() => expect(sent[0]).toBe("quality quarter"));
  });

  it("falls back to full for a damaged or blocked entry", async () => {
    window.localStorage.setItem("preview.quality.v1", "ultra");
    await load();
    expect(store.usePreviewStore.getState().quality).toBe("full");

    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    await load();
    expect(store.usePreviewStore.getState().quality).toBe("full");
  });
});

describe("preview zoom", () => {
  it("steps from fit by the fit scale of the reported area", async () => {
    store.reportPreviewAreaSize({ width: 960, height: 540 }); // fit = 50 %
    store.zoomPreview(1);
    expect(store.usePreviewStore.getState().zoom).toEqual({ kind: "percent", percent: 100 });
    store.zoomPreview(1);
    store.zoomPreview(-1);
    store.zoomPreview(-1);
    store.zoomPreview(-1);
    await vi.waitFor(() =>
      expect(sent).toEqual([
        "view 100 0.000 0.000",
        "view 200 0.000 0.000",
        "view 100 0.000 0.000",
        "view 50 0.000 0.000",
        "view 25 0.000 0.000",
      ]),
    );
  });

  it("keeps the point under the pointer for Ctrl+wheel", () => {
    store.reportPreviewAreaSize({ width: 960, height: 540 });
    store.zoomPreview(1, { x: 240, y: 0 });
    // At 50 % the pointer is over the canvas point 0.25 widths right of the centre; at 100 % that
    // point stays 240 px right of the centre: the centre shows 0.25 - 240/1920.
    expect(store.usePreviewStore.getState().pan.x).toBeCloseTo(0.125);
  });

  it("drops the pan at fit", async () => {
    store.setPreviewZoom({ kind: "percent", percent: 200 });
    store.dragPreviewPan({ x: 0, y: 0 }, -384, 0);
    expect(store.usePreviewStore.getState().pan.x).toBeCloseTo(0.1);
    store.setPreviewZoom({ kind: "fit" });
    expect(store.usePreviewStore.getState().pan).toEqual({ x: 0, y: 0 });
    await vi.waitFor(() => expect(sent.at(-1)).toBe("view fit 0 0"));
  });

  it("sends a pan drag at most once per animation frame with the latest pan", async () => {
    store.setPreviewZoom({ kind: "percent", percent: 100 });
    await vi.waitFor(() => expect(sent).toEqual(["view 100 0.000 0.000"]));
    store.dragPreviewPan({ x: 0, y: 0 }, -192, 0);
    store.dragPreviewPan({ x: 0, y: 0 }, -384, 108);
    expect(frames).toHaveLength(1);
    nextFrame();
    await vi.waitFor(() => expect(sent).toEqual(["view 100 0.000 0.000", "view 100 0.200 -0.100"]));
  });

  it("does not pan at fit", () => {
    store.dragPreviewPan({ x: 0, y: 0 }, 500, 500);
    expect(store.usePreviewStore.getState().pan).toEqual({ x: 0, y: 0 });
    expect(frames).toHaveLength(0);
  });
});

describe("full screen", () => {
  it("toggles the main window full screen and leaves it", () => {
    store.toggleFullScreen();
    expect(store.usePreviewStore.getState().fullScreen).toBe(true);
    expect(setFullscreen).toHaveBeenLastCalledWith(true);

    store.exitFullScreen();
    expect(store.usePreviewStore.getState().fullScreen).toBe(false);
    expect(setFullscreen).toHaveBeenLastCalledWith(false);

    // Leaving when not full screen does nothing.
    store.exitFullScreen();
    expect(setFullscreen).toHaveBeenCalledTimes(2);
  });

  it("shows the layout the window really has when the window refuses", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    setFullscreen.mockRejectedValue(new Error("not allowed"));
    store.toggleFullScreen();
    await vi.waitFor(() => expect(store.usePreviewStore.getState().fullScreen).toBe(false));
  });
});
