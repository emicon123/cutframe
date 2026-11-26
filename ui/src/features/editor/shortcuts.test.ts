import { afterEach, describe, expect, it, vi } from "vitest";

import { playheadStore } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { selectionStore } from "@/stores/selectionStore";
import { useUiStore } from "@/stores/uiStore";

import type { Action } from "./actions";
import {
  findShortcut,
  handleShortcut,
  isInLocalScope,
  isInOverlay,
  isTextEntry,
  type KeyInput,
} from "./shortcuts";

const key = (k: string, mods: Partial<KeyInput> = {}): KeyInput => ({
  key: k,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("findShortcut", () => {
  it("distinguishes undo and redo by Shift", () => {
    expect(findShortcut(key("z", { ctrlKey: true }))?.id).toBe("undo");
    expect(findShortcut(key("Z", { ctrlKey: true, shiftKey: true }))?.id).toBe("redo");
    expect(findShortcut(key("y", { ctrlKey: true }))?.id).toBe("redo");
  });

  it("treats Cmd like Ctrl and ignores Alt combinations", () => {
    expect(findShortcut(key("z", { metaKey: true }))?.id).toBe("undo");
    expect(findShortcut(key("z", { ctrlKey: true, altKey: true }))).toBeUndefined();
  });

  it("finds the file shortcuts", () => {
    expect(findShortcut(key("n", { ctrlKey: true }))?.id).toBe("new-project");
    expect(findShortcut(key("o", { ctrlKey: true }))?.id).toBe("open-project");
    expect(findShortcut(key("s", { ctrlKey: true }))?.id).toBe("save");
    expect(findShortcut(key("S", { ctrlKey: true, shiftKey: true }))?.id).toBe("save-as");
  });

  it("opens the import dialog with Ctrl+I", () => {
    expect(findShortcut(key("i", { ctrlKey: true }))?.id).toBe("import-media");
    expect(findShortcut(key("I", { ctrlKey: true, shiftKey: true }))).toBeUndefined();
  });

  it("finds the timeline edit shortcuts", () => {
    expect(findShortcut(key("b", { ctrlKey: true }))?.id).toBe("split");
    expect(findShortcut(key("Delete"))?.id).toBe("delete");
    expect(findShortcut(key("Delete", { shiftKey: true }))?.id).toBe("ripple-delete");
    expect(findShortcut(key("q"))?.id).toBe("trim-left");
    expect(findShortcut(key("W"))?.id).toBe("trim-right");
    expect(findShortcut(key("x", { ctrlKey: true }))?.id).toBe("cut");
    expect(findShortcut(key("c", { ctrlKey: true }))?.id).toBe("copy");
    expect(findShortcut(key("v", { ctrlKey: true }))?.id).toBe("paste");
    expect(findShortcut(key("d", { ctrlKey: true }))?.id).toBe("duplicate");
    expect(findShortcut(key("q", { ctrlKey: true }))).toBeUndefined();
  });

  it("finds the navigation and zoom shortcuts (F1.54, F1.32)", () => {
    expect(findShortcut(key("ArrowLeft"))?.id).toBe("frame-back");
    expect(findShortcut(key("ArrowRight"))?.id).toBe("frame-forward");
    expect(findShortcut(key("ArrowLeft", { shiftKey: true }))?.id).toBe("frame-back-10");
    expect(findShortcut(key("ArrowRight", { shiftKey: true }))?.id).toBe("frame-forward-10");
    expect(findShortcut(key("ArrowUp"))?.id).toBe("previous-edit");
    expect(findShortcut(key("ArrowDown"))?.id).toBe("next-edit");
    expect(findShortcut(key("Home"))?.id).toBe("go-to-start");
    expect(findShortcut(key("End"))?.id).toBe("go-to-end");
    expect(findShortcut(key("+", { shiftKey: true }))?.id).toBe("zoom-in");
    expect(findShortcut(key("="))?.id).toBe("zoom-in");
    expect(findShortcut(key("-"))?.id).toBe("zoom-out");
    expect(findShortcut(key("Z", { shiftKey: true }))?.id).toBe("zoom-to-fit");
    expect(findShortcut(key("ArrowLeft", { ctrlKey: true }))).toBeUndefined();
  });

  it("ignores plain keys", () => {
    expect(findShortcut(key("z"))).toBeUndefined();
  });

  it("opens the shortcut reference with ? and Ctrl+/, with or without Shift", () => {
    expect(findShortcut(key("?", { shiftKey: true }))?.id).toBe("keyboard-shortcuts");
    expect(findShortcut(key("?"))?.id).toBe("keyboard-shortcuts");
    expect(findShortcut(key("/", { ctrlKey: true }))?.id).toBe("keyboard-shortcuts");
    expect(findShortcut(key("/", { ctrlKey: true, shiftKey: true }))?.id).toBe("keyboard-shortcuts");
    expect(findShortcut(key("/"))).toBeUndefined();
  });
});

describe("isTextEntry", () => {
  it("detects inputs and text areas", () => {
    expect(isTextEntry(document.createElement("input"))).toBe(true);
    expect(isTextEntry(document.createElement("textarea"))).toBe(true);
    expect(isTextEntry(document.createElement("div"))).toBe(false);
    expect(isTextEntry(null)).toBe(false);
  });
});

describe("handleShortcut", () => {
  const state = {
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null, entries: [], current: 0 },
    project: { magnetic: false, tracks: [] },
  } as unknown as ProjectState;

  const press = (k: string, mods: Partial<KeyInput> = {}, target: EventTarget = document.body) => {
    const event = new KeyboardEvent("keydown", { key: k, cancelable: true, ...mods });
    Object.defineProperty(event, "target", { value: target });
    return event;
  };

  const fake = (overrides: Partial<Action>): Action & { run: ReturnType<typeof vi.fn> } =>
    ({
      id: "save",
      label: "Save",
      shortcuts: [{ key: "s", ctrl: true }],
      run: vi.fn(),
      ...overrides,
    }) as Action & { run: ReturnType<typeof vi.fn> };

  afterEach(() => {
    useProjectStore.setState({ state: null });
    useUiStore.setState({ shortcutsOpen: false });
  });

  it("runs an enabled action and blocks the WebView default", () => {
    const action = fake({});
    const event = press("s", { ctrlKey: true });
    handleShortcut(event, [action]);
    expect(action.run).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it("does not run a disabled action", () => {
    useProjectStore.setState({ state });
    const action = fake({ enabled: (ctx) => ctx.canUndo });
    handleShortcut(press("s", { ctrlKey: true }), [action]);
    expect(action.run).not.toHaveBeenCalled();
  });

  it("ignores shortcuts in text fields but still blocks the default where asked", () => {
    const input = document.createElement("input");
    const plain = fake({});
    const blocking = fake({ blockDefault: true });

    const first = press("s", { ctrlKey: true }, input);
    handleShortcut(first, [plain]);
    expect(first.defaultPrevented).toBe(false);

    const second = press("s", { ctrlKey: true }, input);
    handleShortcut(second, [blocking]);
    expect(second.defaultPrevented).toBe(true);
    expect(plain.run).not.toHaveBeenCalled();
    expect(blocking.run).not.toHaveBeenCalled();
  });

  it("leaves an event alone that something already handled", () => {
    const action = fake({ shortcuts: [{ key: "Escape" }] });
    const event = press("Escape");
    event.preventDefault();
    handleShortcut(event, [action]);
    expect(action.run).not.toHaveBeenCalled();
  });

  it("selects every clip with Ctrl+A and deselects with Escape", () => {
    useProjectStore.setState({
      state: {
        ...state,
        project: {
          tracks: [
            { id: "t1", locked: false, clips: [{ id: "a" }, { id: "b" }] },
            { id: "t2", locked: true, clips: [{ id: "c" }] },
          ],
        },
      } as unknown as ProjectState,
    });
    useUiStore.setState({ view: "editor" });
    handleShortcut(press("a", { ctrlKey: true }));
    expect(selectionStore.getState().clips).toEqual(["a", "b"]);
    handleShortcut(press("Escape"));
    expect(selectionStore.getState().clips).toEqual([]);
  });

  it("ignores timeline shortcuts while focus is in a panel with its own keys", () => {
    const panel = document.createElement("section");
    panel.dataset.shortcutScope = "media-bin";
    const button = document.createElement("button");
    panel.append(button);
    const timeline = fake({ shortcuts: [{ key: "Delete" }], scope: "timeline" });
    const global = fake({ shortcuts: [{ key: "Delete" }] });

    const inPanel = press("Delete", {}, button);
    handleShortcut(inPanel, [timeline]);
    expect(timeline.run).not.toHaveBeenCalled();
    expect(inPanel.defaultPrevented).toBe(false);

    handleShortcut(press("Delete", {}, button), [global]);
    expect(global.run).toHaveBeenCalledOnce();
    handleShortcut(press("Delete"), [timeline]);
    expect(timeline.run).toHaveBeenCalledOnce();
  });

  it("leaves the arrow keys to the media bin and to a focused slider", () => {
    const panel = document.createElement("section");
    panel.dataset.shortcutScope = "media-bin";
    const item = document.createElement("div");
    panel.append(item);
    useProjectStore.setState({
      state: {
        ...state,
        project: { duration: 705_600_000, settings: { frameRate: { num: 25, den: 1 } }, tracks: [] },
      } as unknown as ProjectState,
    });
    useUiStore.setState({ view: "editor" });
    playheadStore.setState({ time: 0 });

    const inBin = press("ArrowRight", {}, item);
    handleShortcut(inBin);
    expect(inBin.defaultPrevented).toBe(false);
    expect(playheadStore.getState().time).toBe(0);

    const onSlider = press("ArrowRight");
    onSlider.preventDefault();
    handleShortcut(onSlider);
    expect(playheadStore.getState().time).toBe(0);

    handleShortcut(press("End"));
    expect(playheadStore.getState().time).toBeGreaterThan(0);
  });

  it("does not trim with Q or W while typing", () => {
    const action = fake({ shortcuts: [{ key: "q" }], scope: "timeline" });
    handleShortcut(press("q", {}, document.createElement("input")), [action]);
    expect(action.run).not.toHaveBeenCalled();
  });

  it("opens the shortcut reference on ? from the registry", () => {
    handleShortcut(press("?", { shiftKey: true }));
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
  });

  it("does not open the shortcut reference while typing ?", () => {
    handleShortcut(press("?", { shiftKey: true }, document.createElement("textarea")));
    expect(useUiStore.getState().shortcutsOpen).toBe(false);
  });
});

describe("isInLocalScope", () => {
  it("detects elements inside a data-shortcut-scope panel", () => {
    const panel = document.createElement("div");
    panel.setAttribute("data-shortcut-scope", "media-bin");
    const item = document.createElement("div");
    panel.append(item);
    expect(isInLocalScope(item)).toBe(true);
    expect(isInLocalScope(document.body)).toBe(false);
    expect(isInLocalScope(null)).toBe(false);
  });
});

describe("isInOverlay", () => {
  it("detects elements inside dialogs and menus", () => {
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    const button = document.createElement("button");
    dialog.append(button);
    expect(isInOverlay(button)).toBe(true);
    expect(isInOverlay(document.createElement("button"))).toBe(false);
    expect(isInOverlay(null)).toBe(false);
  });
});
