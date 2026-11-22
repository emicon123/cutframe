import { describe, expect, it } from "vitest";

import { findShortcut, isInOverlay, isTextEntry, type KeyInput, shortcutLabel } from "./shortcuts";

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
    expect(findShortcut(key("y", { ctrlKey: true }))?.id).toBe("redo-alt");
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

  it("ignores plain keys", () => {
    expect(findShortcut(key("z"))).toBeUndefined();
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

describe("shortcutLabel", () => {
  it("formats modifiers and the key", () => {
    expect(shortcutLabel("save")).toBe("Ctrl+S");
    expect(shortcutLabel("save-as")).toBe("Ctrl+Shift+S");
    expect(shortcutLabel("missing")).toBe("");
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
