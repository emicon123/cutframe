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
      [t.file.new, "Ctrl+N"],
      [t.file.open, "Ctrl+O"],
      [t.file.save, "Ctrl+S"],
      [t.file.saveAs, "Ctrl+Shift+S"],
      [t.file.importMedia, "Ctrl+I"],
      [t.file.projectSettings, ""],
      [t.file.exit, ""],
    ]);
    expect(file?.items.filter((i) => i.kind === "separator")).toHaveLength(4);
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
    expect(disabled).toEqual(["save", "save-as", "import-media", "project-settings"]);
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
    expect(actionContext(null, { historyPanelOpen: true, mediaBinOpen: false, inspectorOpen: true })).toEqual(
      ctx({ loaded: false, historyPanelOpen: true, mediaBinOpen: false }),
    );
  });
});
