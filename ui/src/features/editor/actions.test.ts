import { describe, expect, it, vi } from "vitest";

import { t } from "@/i18n";

import {
  type Action,
  type ActionContext,
  ACTIONS,
  actionContext,
  actionTooltip,
  bindingLabel,
  getAction,
  type Menu,
  MENUS,
  menuModel,
  SEPARATOR,
  shortcutGroups,
  shortcutLabel,
} from "./actions";

const ctx = (overrides: Partial<ActionContext> = {}): ActionContext => ({
  loaded: true,
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  redoLabel: null,
  historyPanelOpen: false,
  mediaBinOpen: true,
  inspectorOpen: true,
  editorOpen: true,
  magnetic: true,
  clipsSelected: false,
  gapSelected: false,
  ...overrides,
});

const actionItems = (menu: ReturnType<typeof menuModel>[number] | undefined) =>
  (menu?.items ?? []).flatMap((item) => (item.kind === "action" ? [item] : []));

describe("registry", () => {
  it("registers every action once and shows it in at least one menu", () => {
    const ids = ACTIONS.map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
    const inMenus = new Set(MENUS.flatMap((m) => m.items));
    for (const id of ids) expect(inMenus.has(id)).toBe(true);
  });

  it("uses each key combination for one action only", () => {
    const combos = ACTIONS.flatMap((a) => a.shortcuts ?? []).map(bindingLabel);
    expect(new Set(combos).size).toBe(combos.length);
  });
});

describe("bindingLabel", () => {
  it("formats modifiers and the key", () => {
    expect(shortcutLabel(getAction("save"))).toBe("Ctrl+S");
    expect(shortcutLabel(getAction("save-as"))).toBe("Ctrl+Shift+S");
    expect(shortcutLabel(getAction("exit"))).toBe("");
    expect(bindingLabel({ key: "?", shift: "any" })).toBe("?");
    expect(bindingLabel({ key: "/", ctrl: true, shift: "any" })).toBe("Ctrl+/");
  });
});

describe("menuModel", () => {
  const menus = menuModel(ctx());

  it("builds File, Edit, View and Help in order", () => {
    expect(menus.map((m) => m.label)).toEqual([t.menu.file, t.menu.edit, t.menu.view, t.menu.help]);
  });

  it("shows the file actions with their shortcuts and separators", () => {
    const file = menus[0];
    expect(actionItems(file).map((i) => [i.label, i.shortcut])).toEqual([
      [t.file.home, ""],
      [t.file.new, "Ctrl+N"],
      [t.file.open, "Ctrl+O"],
      [t.file.save, "Ctrl+S"],
      [t.file.saveAs, "Ctrl+Shift+S"],
      [t.file.importMedia, "Ctrl+I"],
      [t.file.projectSettings, ""],
      [t.file.exit, ""],
    ]);
    expect(file?.items.filter((i) => i.kind === "separator")).toHaveLength(5);
  });

  it("names the undo and redo entries and disables them when there is nothing to do", () => {
    const idle = actionItems(menus[1]);
    expect(idle.find((i) => i.id === "undo")).toMatchObject({ label: t.edit.undo, disabled: true });
    expect(idle.find((i) => i.id === "redo")).toMatchObject({ label: t.edit.redo, disabled: true });

    const busy = actionItems(
      menuModel(ctx({ canUndo: true, undoLabel: "addTrack", canRedo: true, redoLabel: "renameTrack" }))[1],
    );
    expect(busy.find((i) => i.id === "undo")).toMatchObject({
      label: t.edit.undoAction(t.history.labels.addTrack),
      shortcut: "Ctrl+Z",
      disabled: false,
    });
    expect(busy.find((i) => i.id === "redo")).toMatchObject({
      label: t.edit.redoAction(t.history.labels.renameTrack),
      shortcut: "Ctrl+Shift+Z",
      disabled: false,
    });
  });

  it("disables project actions before a project is loaded", () => {
    const file = actionItems(menuModel(ctx({ loaded: false }))[0]);
    const disabled = file.filter((i) => i.disabled).map((i) => i.id);
    expect(disabled).toEqual(["go-home", "save", "save-as", "import-media", "project-settings"]);
  });

  it("checks the history panel toggle when the panel is open", () => {
    const find = (c: ActionContext) => actionItems(menuModel(c)[2]).find((i) => i.id === "history-panel");
    expect(find(ctx())?.checked).toBe(false);
    expect(find(ctx({ historyPanelOpen: true }))?.checked).toBe(true);
    expect(actionItems(menus[0])[0]?.checked).toBeUndefined();
  });

  it("generates a menu from any registry", () => {
    const action: Action = {
      id: "save",
      label: "Write",
      shortcuts: [{ key: "w", ctrl: true }],
      run: vi.fn(),
    };
    const custom: Menu[] = [{ id: "file", label: "F", items: ["save", SEPARATOR] }];
    expect(menuModel(ctx(), custom, [action])).toEqual([
      {
        id: "file",
        label: "F",
        items: [
          {
            kind: "action",
            id: "save",
            label: "Write",
            shortcut: "Ctrl+W",
            disabled: false,
            checked: undefined,
          },
          { kind: "separator" },
        ],
      },
    ]);
  });
});

describe("shortcutGroups", () => {
  it("lists every shortcut once, grouped by the first menu that has it", () => {
    const groups = shortcutGroups();
    expect(groups.map((g) => g.label)).toEqual([t.menu.file, t.menu.edit, t.menu.view]);
    expect(groups[1]?.rows).toEqual([
      { id: "undo", label: t.edit.undo, keys: ["Ctrl+Z"] },
      { id: "redo", label: t.edit.redo, keys: ["Ctrl+Shift+Z", "Ctrl+Y"] },
      { id: "cut", label: t.edit.cut, keys: ["Ctrl+X"] },
      { id: "copy", label: t.edit.copy, keys: ["Ctrl+C"] },
      { id: "paste", label: t.edit.paste, keys: ["Ctrl+V"] },
      { id: "duplicate", label: t.edit.duplicate, keys: ["Ctrl+D"] },
      { id: "split", label: t.edit.split, keys: ["Ctrl+B"] },
      { id: "delete", label: t.edit.delete, keys: ["Del"] },
      { id: "ripple-delete", label: t.edit.rippleDelete, keys: ["Shift+Del"] },
      { id: "trim-left", label: t.edit.trimLeft, keys: ["Q"] },
      { id: "trim-right", label: t.edit.trimRight, keys: ["W"] },
      { id: "select-all", label: t.edit.selectAll, keys: ["Ctrl+A"] },
      { id: "deselect", label: t.edit.deselect, keys: [t.keys.escape] },
      { id: "preferences", label: t.edit.preferences, keys: ["Ctrl+,"] },
    ]);
    expect(groups[2]?.rows).toEqual([
      { id: "keyboard-shortcuts", label: t.view.keyboardShortcuts, keys: ["?", "Ctrl+/"] },
    ]);
    const listed = groups.flatMap((g) => g.rows.map((r) => r.id));
    const withShortcuts = ACTIONS.filter((a) => a.shortcuts?.length).map((a) => a.id);
    expect([...listed].sort()).toEqual([...withShortcuts].sort());
  });
});

describe("actionTooltip", () => {
  it("shows the label and the shortcut", () => {
    expect(actionTooltip(getAction("undo"), ctx({ canUndo: true, undoLabel: "addTrack" }))).toBe(
      `${t.edit.undoAction(t.history.labels.addTrack)} (Ctrl+Z)`,
    );
    expect(actionTooltip(getAction("exit"), ctx())).toBe(t.file.exit);
  });
});

describe("panel and preferences actions", () => {
  it("checks the side panel toggles while the panels are open", () => {
    const view = (c: ActionContext) => actionItems(menuModel(c)[2]);
    const checked = (c: ActionContext, id: string) => view(c).find((i) => i.id === id)?.checked;
    expect(checked(ctx(), "toggle-media-bin")).toBe(true);
    expect(checked(ctx({ mediaBinOpen: false }), "toggle-media-bin")).toBe(false);
    expect(checked(ctx({ inspectorOpen: false }), "toggle-inspector")).toBe(false);
    expect(view(ctx()).find((i) => i.id === "reset-panel-layout")).toMatchObject({ checked: undefined });
  });

  it("opens Preferences from the Edit menu with Ctrl+,", () => {
    const edit = actionItems(menuModel(ctx())[1]);
    expect(edit.at(-1)).toMatchObject({ id: "preferences", label: t.edit.preferences, shortcut: "Ctrl+," });
  });
});

describe("actionContext", () => {
  it("is empty before the first project state", () => {
    expect(
      actionContext(null, {
        historyPanelOpen: true,
        mediaBinOpen: false,
        inspectorOpen: true,
        editorOpen: true,
      }),
    ).toEqual(ctx({ loaded: false, historyPanelOpen: true, mediaBinOpen: false, magnetic: false }));
  });
});

describe("home screen", () => {
  it("File → Home is the first File item, has no shortcut and needs the editor", () => {
    const file = menuModel(ctx())[0];
    expect(file?.items[0]).toMatchObject({ kind: "action", id: "go-home", label: t.file.home, shortcut: "" });
    expect(file?.items[1]).toEqual({ kind: "separator" });
    const onHome = actionItems(menuModel(ctx({ editorOpen: false }))[0]);
    const disabled = (id: string) => onHome.find((item) => item.id === id)?.disabled;
    expect(disabled("go-home")).toBe(true);
    expect(disabled("save")).toBe(true);
    expect(disabled("import-media")).toBe(true);
    expect(disabled("new-project")).toBe(false);
    expect(disabled("open-project")).toBe(false);
  });
});

describe("timeline edit actions", () => {
  const edit = (c: ActionContext) => actionItems(menuModel(c)[1]);
  const item = (c: ActionContext, id: string) => edit(c).find((i) => i.id === id);

  it("lists split, delete, trim and clipboard actions in the Edit menu with their shortcuts", () => {
    const shortcuts = Object.fromEntries(edit(ctx()).map((i) => [i.id, i.shortcut]));
    expect(shortcuts).toMatchObject({
      split: "Ctrl+B",
      delete: "Del",
      "ripple-delete": "Shift+Del",
      "trim-left": "Q",
      "trim-right": "W",
      cut: "Ctrl+X",
      copy: "Ctrl+C",
      paste: "Ctrl+V",
      duplicate: "Ctrl+D",
      magnetic: "",
    });
  });

  it("needs a selection for delete and the clip clipboard actions, not for split, Q, W or paste", () => {
    const none = ctx();
    for (const id of ["delete", "ripple-delete", "cut", "copy", "duplicate"])
      expect(item(none, id)?.disabled).toBe(true);
    for (const id of ["split", "trim-left", "trim-right", "paste", "magnetic"])
      expect(item(none, id)?.disabled).toBe(false);
    const clips = ctx({ clipsSelected: true });
    for (const id of ["delete", "ripple-delete", "cut", "copy", "duplicate"])
      expect(item(clips, id)?.disabled).toBe(false);
    const gap = ctx({ gapSelected: true });
    expect(item(gap, "delete")?.disabled).toBe(false);
    expect(item(gap, "copy")?.disabled).toBe(true);
  });

  it("disables every edit on the home screen", () => {
    const home = ctx({ editorOpen: false, clipsSelected: true });
    for (const id of ["split", "delete", "trim-left", "copy", "paste", "magnetic"])
      expect(item(home, id)?.disabled).toBe(true);
  });

  it("shows the magnetic main track as a toggle", () => {
    expect(item(ctx({ magnetic: true }), "magnetic")?.checked).toBe(true);
    expect(item(ctx({ magnetic: false }), "magnetic")?.checked).toBe(false);
  });

  it("lists the new shortcuts in the reference panel under Edit", () => {
    const rows = shortcutGroups().find((g) => g.id === "edit")?.rows ?? [];
    expect(rows.find((r) => r.id === "ripple-delete")?.keys).toEqual(["Shift+Del"]);
    expect(rows.find((r) => r.id === "split")?.label).toBe(t.edit.split);
  });
});
