import { afterEach, describe, expect, it, vi } from "vitest";
import type { PanelImperativeHandle } from "react-resizable-panels";

import { useUiStore } from "@/stores/uiStore";

import { getAction } from "./actions";
import {
  LAYOUT_IDS,
  layoutStorage,
  PANEL_REFS,
  removeSavedLayouts,
  resetPanelLayout,
  safeLayoutStorage,
  togglePanel,
} from "./layout";

const throwing = (): Storage => {
  throw new DOMException("blocked", "SecurityError");
};

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("safeLayoutStorage", () => {
  it("passes valid layouts through", () => {
    const storage = safeLayoutStorage(() => localStorage);
    storage.setItem("k", JSON.stringify({ a: 30, b: 70 }));
    expect(storage.getItem("k")).toBe('{"a":30,"b":70}');
  });

  it("survives a storage that cannot be reached", () => {
    const storage = safeLayoutStorage(throwing);
    expect(storage.getItem("k")).toBeNull();
    expect(() => storage.setItem("k", "{}")).not.toThrow();
  });

  it("survives a storage whose methods throw (full or blocked)", () => {
    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new DOMException("full", "QuotaExceededError");
      },
    } as unknown as Storage;
    const storage = safeLayoutStorage(() => broken);
    expect(storage.getItem("k")).toBeNull();
    expect(() => storage.setItem("k", "{}")).not.toThrow();
  });

  it("treats a damaged entry as no layout", () => {
    const storage = safeLayoutStorage(() => localStorage);
    for (const bad of ["not json", "[1,2]", '{"a":"wide"}', "null", "42"]) {
      localStorage.setItem("k", bad);
      expect(storage.getItem("k")).toBeNull();
    }
  });
});

describe("reset panel layout", () => {
  it("removes the editor layouts of every version and nothing else", () => {
    localStorage.setItem(`react-resizable-panels:${LAYOUT_IDS.rows}`, "{}");
    localStorage.setItem(`react-resizable-panels:${LAYOUT_IDS.columns}`, "{}");
    localStorage.setItem("react-resizable-panels:layout.editor.v0.rows", "{}");
    localStorage.setItem("other", "kept");
    expect(removeSavedLayouts(() => localStorage)).toBe(true);
    expect(localStorage.length).toBe(1);
    expect(localStorage.getItem("other")).toBe("kept");
  });

  it("reports a storage that cannot be reached", () => {
    expect(removeSavedLayouts(throwing)).toBe(false);
  });

  it("remounts the panels after clearing", () => {
    layoutStorage.setItem(`x:${LAYOUT_IDS.rows}`, '{"a":1}');
    const before = useUiStore.getState().layoutVersion;
    getAction("reset-panel-layout").run();
    resetPanelLayout();
    expect(useUiStore.getState().layoutVersion).toBe(before + 2);
    expect(localStorage.getItem(`x:${LAYOUT_IDS.rows}`)).toBeNull();
  });
});

describe("panel toggles", () => {
  function fakePanel(collapsed: boolean) {
    const state = { collapsed };
    const handle: PanelImperativeHandle = {
      collapse: vi.fn(() => (state.collapsed = true)),
      expand: vi.fn(() => (state.collapsed = false)),
      getSize: () => ({ asPercentage: state.collapsed ? 0 : 20, inPixels: 0 }),
      isCollapsed: () => state.collapsed,
      resize: vi.fn(),
    };
    return { handle, state };
  }

  it("collapses an open panel and expands a collapsed one", () => {
    const media = fakePanel(false);
    const cleanup = PANEL_REFS.mediaBin(media.handle);
    getAction("toggle-media-bin").run();
    expect(media.handle.collapse).toHaveBeenCalledTimes(1);
    getAction("toggle-media-bin").run();
    expect(media.handle.expand).toHaveBeenCalledTimes(1);
    cleanup?.();
  });

  it("toggles only the named panel and does nothing once unmounted", () => {
    const media = fakePanel(false);
    const inspector = fakePanel(true);
    const stopMedia = PANEL_REFS.mediaBin(media.handle);
    const stopInspector = PANEL_REFS.inspector(inspector.handle);
    getAction("toggle-inspector").run();
    expect(inspector.handle.expand).toHaveBeenCalledTimes(1);
    expect(media.handle.collapse).not.toHaveBeenCalled();
    stopMedia?.();
    stopInspector?.();
    togglePanel("mediaBin");
    expect(media.handle.collapse).not.toHaveBeenCalled();
  });
});
