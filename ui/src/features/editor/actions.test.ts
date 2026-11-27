import { describe, expect, it, vi } from "vitest";

import { t } from "@/i18n";
import type { ProjectState } from "@/stores/projectStore";

import {
  type Action,
  type ActionContext,
  ACTIONS,
  actionContext,
  actionTooltip,
  bindingLabel,
  clipFlags,
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
  linkToggle: null,
  canDetach: false,
  canReverse: false,
  reversed: false,
  canPlay: true,
  playing: false,
  looping: false,
  exporting: false,
  previewingMedia: false,
  fullScreen: false,
  previewQuality: "full",
  skimming: false,
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

  it("uses each key combination for one action only, except bindings that depend on state", () => {
    const combos = ACTIONS.flatMap((a) => a.shortcuts ?? [])
      .filter((binding) => !binding.when)
      .map(bindingLabel);
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

  it("names the navigation keys", () => {
    expect(shortcutLabel(getAction("frame-back"))).toBe("Left");
    expect(shortcutLabel(getAction("frame-forward-10"))).toBe("Shift+Right");
    expect(shortcutLabel(getAction("previous-edit"))).toBe("Up");
    expect(shortcutLabel(getAction("next-edit"))).toBe("Down");
    expect(shortcutLabel(getAction("go-to-start"))).toBe("Home");
    expect(shortcutLabel(getAction("go-to-end"))).toBe("End");
    expect(shortcutLabel(getAction("zoom-in"))).toBe("+");
    expect(shortcutLabel(getAction("zoom-to-fit"))).toBe("Shift+Z");
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
      [t.file.export, "Ctrl+E"],
      [t.file.projectSettings, ""],
      [t.file.exit, ""],
    ]);
    expect(file?.items.filter((i) => i.kind === "separator")).toHaveLength(6);
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
    expect(disabled).toEqual(["go-home", "save", "save-as", "import-media", "export", "project-settings"]);
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
      { id: "freeze-frame", label: t.edit.freezeFrame, keys: ["Shift+F"] },
      { id: "select-all", label: t.edit.selectAll, keys: ["Ctrl+A"] },
      { id: "deselect", label: t.edit.deselect, keys: [t.keys.escape] },
      { id: "preferences", label: t.edit.preferences, keys: ["Ctrl+,"] },
    ]);
    expect(groups[2]?.rows).toEqual([
      { id: "play-pause", label: t.playback.playPause, keys: [t.keys.space] },
      { id: "shuttle-reverse", label: t.playback.shuttleReverse, keys: ["J"] },
      { id: "shuttle-pause", label: t.playback.shuttlePause, keys: ["K"] },
      { id: "shuttle-forward", label: t.playback.shuttleForward, keys: ["L"] },
      { id: "back-to-timeline", label: t.playback.backToTimeline, keys: [t.keys.escape] },
      { id: "full-screen", label: t.view.fullScreen, keys: ["F", t.keys.escape] },
      { id: "preview-zoom-in", label: t.view.previewZoomIn, keys: ["Ctrl++", "Ctrl+="] },
      { id: "preview-zoom-out", label: t.view.previewZoomOut, keys: ["Ctrl+-"] },
      { id: "preview-zoom-fit", label: t.view.previewZoomFit, keys: ["Ctrl+0"] },
      { id: "zoom-in", label: t.view.zoomIn, keys: ["+", "="] },
      { id: "zoom-out", label: t.view.zoomOut, keys: ["-"] },
      { id: "zoom-to-fit", label: t.view.zoomToFit, keys: ["Shift+Z"] },
      { id: "go-to-start", label: t.view.goToStart, keys: ["Home"] },
      { id: "go-to-end", label: t.view.goToEnd, keys: ["End"] },
      { id: "previous-edit", label: t.view.previousEdit, keys: ["Up"] },
      { id: "next-edit", label: t.view.nextEdit, keys: ["Down"] },
      { id: "frame-back", label: t.view.frameBack, keys: ["Left"] },
      { id: "frame-forward", label: t.view.frameForward, keys: ["Right"] },
      { id: "frame-back-10", label: t.view.frameBack10, keys: ["Shift+Left"] },
      { id: "frame-forward-10", label: t.view.frameForward10, keys: ["Shift+Right"] },
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

describe("export action", () => {
  const item = (c: ActionContext) => actionItems(menuModel(c)[0]).find((i) => i.id === "export");

  it("is File → Export… with Ctrl+E and opens the dialog", async () => {
    const { useExportStore } = await import("@/features/export/exportStore");
    expect(item(ctx())).toMatchObject({ label: t.file.export, shortcut: "Ctrl+E", disabled: false });
    expect(getAction("export").blockDefault).toBe(true);
    getAction("export").run();
    expect(useExportStore.getState().dialogOpen).toBe(true);
    useExportStore.setState({ dialogOpen: false });
  });

  it("is disabled on an empty timeline, on the home screen and while an export runs", () => {
    expect(item(ctx({ canPlay: false }))?.disabled).toBe(true);
    expect(item(ctx({ editorOpen: false }))?.disabled).toBe(true);
    expect(item(ctx({ exporting: true }))?.disabled).toBe(true);
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
        skimming: false,
      }),
    ).toEqual(
      ctx({ loaded: false, historyPanelOpen: true, mediaBinOpen: false, magnetic: false, canPlay: false }),
    );
  });

  it("can play a timeline with content and mirrors the transport flags", () => {
    const state = { project: { duration: 1, magnetic: true }, history: {} } as unknown as ProjectState;
    const c = actionContext(state, undefined, undefined, undefined, {
      playing: true,
      looping: true,
      previewingMedia: false,
    });
    expect(c).toMatchObject({ canPlay: true, playing: true, looping: true });
    const empty = { project: { duration: 0, magnetic: true }, history: {} } as unknown as ProjectState;
    expect(actionContext(empty).canPlay).toBe(false);
  });
});

describe("transport actions", () => {
  it("labels play/pause by the transport state and checks the loop toggle", () => {
    const item = (c: ActionContext, id: string) => actionItems(menuModel(c)[2]).find((i) => i.id === id);
    expect(item(ctx(), "play-pause")).toMatchObject({ label: t.playback.play, shortcut: t.keys.space });
    expect(item(ctx({ playing: true }), "play-pause")?.label).toBe(t.playback.pause);
    expect(item(ctx(), "loop")).toMatchObject({ checked: false, shortcut: "" });
    expect(item(ctx({ looping: true }), "loop")?.checked).toBe(true);
  });

  it("cannot start an empty timeline but can always pause", () => {
    const item = (c: ActionContext, id: string) => actionItems(menuModel(c)[2]).find((i) => i.id === id);
    const empty = ctx({ canPlay: false });
    expect(item(empty, "play-pause")?.disabled).toBe(true);
    expect(item(empty, "shuttle-forward")?.disabled).toBe(true);
    expect(item(empty, "shuttle-pause")?.disabled).toBe(false);
    expect(item({ ...empty, playing: true }, "play-pause")?.disabled).toBe(false);
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

  it("labels the link toggle by what it would do and enables it only when it applies", () => {
    expect(item(ctx(), "link-toggle")).toMatchObject({ label: t.edit.link, disabled: true, shortcut: "" });
    expect(item(ctx({ linkToggle: "link" }), "link-toggle")).toMatchObject({
      label: t.edit.link,
      disabled: false,
    });
    expect(item(ctx({ linkToggle: "unlink" }), "link-toggle")).toMatchObject({
      label: t.edit.unlink,
      disabled: false,
    });
    expect(item(ctx({ linkToggle: "link", editorOpen: false }), "link-toggle")?.disabled).toBe(true);
  });

  it("enables detach audio only for clips that still make sound", () => {
    expect(item(ctx({ clipsSelected: true }), "detach-audio")?.disabled).toBe(true);
    expect(item(ctx({ clipsSelected: true, canDetach: true }), "detach-audio")?.disabled).toBe(false);
  });

  it("offers freeze frame with Shift+F and reverse as a toggle in the Edit menu (F1.81, F1.82)", () => {
    const freeze = item(ctx(), "freeze-frame");
    expect(freeze).toMatchObject({ label: t.edit.freezeFrame, shortcut: "Shift+F", disabled: false });
    expect(item(ctx({ editorOpen: false }), "freeze-frame")?.disabled).toBe(true);
    expect(item(ctx({ clipsSelected: true }), "reverse")).toMatchObject({ disabled: true, checked: false });
    expect(item(ctx({ clipsSelected: true, canReverse: true, reversed: true }), "reverse")).toMatchObject({
      disabled: false,
      checked: true,
    });
  });

  it("lists the new shortcuts in the reference panel under Edit", () => {
    const rows = shortcutGroups().find((g) => g.id === "edit")?.rows ?? [];
    expect(rows.find((r) => r.id === "ripple-delete")?.keys).toEqual(["Shift+Del"]);
    expect(rows.find((r) => r.id === "split")?.label).toBe(t.edit.split);
  });
});

describe("clipFlags", () => {
  const state = {
    project: {
      tracks: [
        {
          id: "v",
          kind: "video",
          clips: [
            {
              id: "a",
              duration: 1,
              link: null,
              audio: {},
              source: { kind: "media", media: "m", sourceIn: 0, speed: 100, reverse: false },
            },
            { id: "i", duration: 1, link: null, source: { kind: "media", media: "img", sourceIn: 0 } },
          ],
        },
        {
          id: "s",
          kind: "audio",
          clips: [
            {
              id: "b",
              duration: 1,
              link: null,
              audio: {},
              source: { kind: "media", media: "m", sourceIn: 0, speed: 100, reverse: true },
            },
          ],
        },
      ],
      media: [
        { id: "m", kind: "video" },
        { id: "img", kind: "image" },
      ],
    },
  } as unknown as ProjectState;

  it("derives the link toggle and detach from the selected clips", () => {
    expect(clipFlags(state, ["a", "b"])).toMatchObject({ linkToggle: "link", canDetach: true });
    expect(clipFlags(state, ["b"])).toMatchObject({ linkToggle: null, canDetach: false });
    expect(clipFlags(null, ["a"])).toEqual({
      linkToggle: null,
      canDetach: false,
      canReverse: false,
      reversed: false,
    });
  });

  it("offers reverse for video and audio clips, checked when all of them play reversed (F1.81)", () => {
    expect(clipFlags(state, ["a", "b"])).toMatchObject({ canReverse: true, reversed: false });
    expect(clipFlags(state, ["b", "i"])).toMatchObject({ canReverse: true, reversed: true });
    expect(clipFlags(state, ["i"])).toMatchObject({ canReverse: false, reversed: false });
  });
});

describe("preview actions (F1.55, F1.61-F1.63, F1.17)", () => {
  const view = (c: ActionContext) => actionItems(menuModel(c)[2]);
  const item = (c: ActionContext, id: string) => view(c).find((i) => i.id === id);

  it("checks the quality in use", () => {
    const half = ctx({ previewQuality: "half" });
    expect(item(half, "preview-quality-half")?.checked).toBe(true);
    expect(item(half, "preview-quality-full")?.checked).toBe(false);
    expect(item(half, "preview-quality-quarter")).toMatchObject({ label: t.view.previewQuality.quarter });
  });

  it("labels full screen by state, with F", () => {
    expect(item(ctx(), "full-screen")).toMatchObject({
      label: t.view.fullScreen,
      shortcut: "F",
      checked: false,
    });
    expect(item(ctx({ fullScreen: true }), "full-screen")).toMatchObject({
      label: t.view.exitFullScreen,
      checked: true,
    });
  });

  it("shows the skimming toggle and the preview zoom shortcuts", () => {
    expect(item(ctx({ skimming: true }), "skimming")?.checked).toBe(true);
    expect(item(ctx(), "skimming")?.checked).toBe(false);
    expect(item(ctx(), "preview-zoom-in")?.shortcut).toBe("Ctrl++");
    expect(item(ctx(), "preview-zoom-fit")?.shortcut).toBe("Ctrl+0");
  });

  it("plays a previewed media item even on an empty timeline and goes back only from one", () => {
    const empty = ctx({ canPlay: false, previewingMedia: true });
    expect(item(empty, "play-pause")?.disabled).toBe(false);
    expect(item(empty, "shuttle-forward")?.disabled).toBe(false);
    expect(item(empty, "back-to-timeline")?.disabled).toBe(false);
    expect(item(ctx(), "back-to-timeline")?.disabled).toBe(true);
  });
});
