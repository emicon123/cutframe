import { useStore } from "zustand";
import { useShallow } from "zustand/react/shallow";

import { type ClipId, commands } from "@/bindings";
import { importFromDialog } from "@/features/media-bin/importActions";
import { openLogsFolder } from "@/features/notifications/logs";
import { fileActions } from "@/features/project/fileActions";
import { checkForUpdates } from "@/features/updater/updater";
import {
  copySelection,
  cutSelection,
  deleteSelection,
  detachSelectedAudio,
  duplicateSelection,
  pasteAtPlayhead,
  splitAtPlayhead,
  toggleLinkSelection,
  toggleMagnetic,
  trimToPlayhead,
} from "@/features/timeline/editActions";
import { canDetachAudio, linkToggle, type LinkToggle } from "@/features/timeline/linking";
import {
  fitTimeline,
  goToEnd,
  goToStart,
  jumpToEdit,
  stepFrames,
  zoomStep,
} from "@/features/timeline/navActions";
import { historyLabelText, t } from "@/i18n";
import { run } from "@/stores/appStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { clearSelection, selectAll, selectionStore } from "@/stores/selectionStore";
import { useUiStore } from "@/stores/uiStore";

import { resetPanelLayout, togglePanel } from "./layout";

/** A key combination. Ctrl also matches Cmd; any combination with Alt never matches. */
export type KeyBinding = {
  /** `KeyboardEvent.key`, compared case-insensitively. */
  key: string;
  ctrl?: boolean;
  /**
   * `"any"` for a symbol whose key needs Shift on some layouts (`?`, `/`), so the binding matches
   * with or without it and the label does not show Shift.
   */
  shift?: boolean | "any";
};

/** What actions read to decide their menu label, enabled and checked state. */
export type ActionContext = {
  loaded: boolean;
  canUndo: boolean;
  canRedo: boolean;
  /** `HistoryLabel` key of the entry undo would revert. */
  undoLabel: string | null;
  redoLabel: string | null;
  historyPanelOpen: boolean;
  mediaBinOpen: boolean;
  inspectorOpen: boolean;
  /** The editor is shown (not the home screen); file and project actions need it. */
  editorOpen: boolean;
  /** The project's main track is magnetic (`ProjectView.magnetic`). */
  magnetic: boolean;
  /** At least one timeline clip is selected. */
  clipsSelected: boolean;
  /** A gap on the timeline is selected. */
  gapSelected: boolean;
  /** What the link toggle would do with the selected clips (F1.49). */
  linkToggle: LinkToggle;
  /** A selected clip still plays its own sound, so detach audio applies (F1.50). */
  canDetach: boolean;
};

/** The selection-store part of the context. */
export type SelectionFlags = Pick<ActionContext, "clipsSelected" | "gapSelected">;

/** The part that depends on both the selection and the clips it names. */
export type ClipFlags = Pick<ActionContext, "linkToggle" | "canDetach">;

/** The UI-store part of the context. */
export type UiFlags = Pick<
  ActionContext,
  "historyPanelOpen" | "mediaBinOpen" | "inspectorOpen" | "editorOpen"
>;

export type ActionId =
  | "go-home"
  | "new-project"
  | "open-project"
  | "save"
  | "save-as"
  | "import-media"
  | "project-settings"
  | "exit"
  | "undo"
  | "redo"
  | "select-all"
  | "deselect"
  | "split"
  | "delete"
  | "ripple-delete"
  | "trim-left"
  | "trim-right"
  | "cut"
  | "copy"
  | "paste"
  | "duplicate"
  | "link-toggle"
  | "detach-audio"
  | "magnetic"
  | "preferences"
  | "history-panel"
  | "toggle-media-bin"
  | "toggle-inspector"
  | "reset-panel-layout"
  | "keyboard-shortcuts"
  | "check-for-updates"
  | "open-logs-folder"
  | "about"
  | "frame-back"
  | "frame-forward"
  | "frame-back-10"
  | "frame-forward-10"
  | "previous-edit"
  | "next-edit"
  | "go-to-start"
  | "go-to-end"
  | "zoom-in"
  | "zoom-out"
  | "zoom-to-fit";

/**
 * A user command. Register it once in `ACTIONS`; the main menu, the shortcut reference panel,
 * tooltips and the keyboard handler are all generated from that entry.
 */
export type Action = {
  id: ActionId;
  /** Static name, used in the shortcut reference panel. */
  label: string;
  /** Menu and tooltip text when it depends on state, e.g. "Undo Add Track". */
  menuLabel?: (ctx: ActionContext) => string;
  /** The first one is shown in menus and tooltips; the reference panel lists all. */
  shortcuts?: readonly KeyBinding[];
  /**
   * Always block the WebView2 default of the shortcuts (Ctrl+N new window, Ctrl+O/S file
   * dialogs), even where the action itself is ignored (text fields, open dialogs).
   */
  blockDefault?: boolean;
  /** Absent: always enabled. A disabled action is greyed in menus and its shortcut does nothing. */
  enabled?: (ctx: ActionContext) => boolean;
  /**
   * `"timeline"`: the shortcut acts on the timeline, so it is ignored while focus is inside a
   * panel with its own keyboard handling (`data-shortcut-scope`, e.g. the media bin, where Delete
   * removes media and Ctrl+C must not copy timeline clips).
   */
  scope?: "timeline";
  /** Present for toggles: the menu shows a check mark. */
  checked?: (ctx: ActionContext) => boolean;
  run: () => void;
};

/** A project is loaded and shown in the editor (the home screen's untitled project is not). */
const editing = (ctx: ActionContext) => ctx.loaded && ctx.editorOpen;
const clipsSelected = (ctx: ActionContext) => editing(ctx) && ctx.clipsSelected;
const ui = () => useUiStore.getState();

/** The single registry of editor actions. */
export const ACTIONS: readonly Action[] = [
  {
    id: "go-home",
    label: t.file.home,
    enabled: editing,
    run: () => void fileActions.goHome(),
  },
  {
    id: "new-project",
    label: t.file.new,
    shortcuts: [{ key: "n", ctrl: true }],
    blockDefault: true,
    run: () => void fileActions.newProject(),
  },
  {
    id: "open-project",
    label: t.file.open,
    shortcuts: [{ key: "o", ctrl: true }],
    blockDefault: true,
    run: () => void fileActions.openProject(),
  },
  {
    id: "save",
    label: t.file.save,
    shortcuts: [{ key: "s", ctrl: true }],
    blockDefault: true,
    enabled: editing,
    run: () => void fileActions.save(),
  },
  {
    id: "save-as",
    label: t.file.saveAs,
    shortcuts: [{ key: "s", ctrl: true, shift: true }],
    blockDefault: true,
    enabled: editing,
    run: () => void fileActions.saveAs(),
  },
  {
    id: "import-media",
    label: t.file.importMedia,
    shortcuts: [{ key: "i", ctrl: true }],
    enabled: editing,
    run: () => void importFromDialog(),
  },
  {
    id: "project-settings",
    label: t.file.projectSettings,
    enabled: editing,
    run: () => ui().setProjectSettingsOpen(true),
  },
  {
    id: "exit",
    label: t.file.exit,
    // The same path as the window's close button: the unsaved-changes prompt, then `confirmClose`.
    run: () => void fileActions.requestClose(),
  },
  {
    id: "undo",
    label: t.edit.undo,
    menuLabel: (ctx) => (ctx.undoLabel ? t.edit.undoAction(historyLabelText(ctx.undoLabel)) : t.edit.undo),
    shortcuts: [{ key: "z", ctrl: true }],
    enabled: (ctx) => ctx.canUndo,
    run: () => void run(commands.undo()),
  },
  {
    id: "redo",
    label: t.edit.redo,
    menuLabel: (ctx) => (ctx.redoLabel ? t.edit.redoAction(historyLabelText(ctx.redoLabel)) : t.edit.redo),
    shortcuts: [
      { key: "z", ctrl: true, shift: true },
      { key: "y", ctrl: true },
    ],
    enabled: (ctx) => ctx.canRedo,
    run: () => void run(commands.redo()),
  },
  {
    id: "select-all",
    label: t.edit.selectAll,
    shortcuts: [{ key: "a", ctrl: true }],
    scope: "timeline",
    enabled: editing,
    run: selectAll,
  },
  {
    id: "deselect",
    label: t.edit.deselect,
    shortcuts: [{ key: "Escape" }],
    run: clearSelection,
  },
  {
    id: "split",
    label: t.edit.split,
    // With no selection, every clip under the playhead on an unlocked track splits.
    shortcuts: [{ key: "b", ctrl: true }],
    scope: "timeline",
    enabled: editing,
    run: () => void splitAtPlayhead(),
  },
  {
    id: "delete",
    label: t.edit.delete,
    shortcuts: [{ key: "Delete" }],
    scope: "timeline",
    enabled: (ctx) => editing(ctx) && (ctx.clipsSelected || ctx.gapSelected),
    run: () => void deleteSelection(false),
  },
  {
    id: "ripple-delete",
    label: t.edit.rippleDelete,
    shortcuts: [{ key: "Delete", shift: true }],
    scope: "timeline",
    enabled: (ctx) => editing(ctx) && (ctx.clipsSelected || ctx.gapSelected),
    run: () => void deleteSelection(true),
  },
  {
    id: "trim-left",
    label: t.edit.trimLeft,
    // With no selection, the main-track clip under the playhead.
    shortcuts: [{ key: "q" }],
    scope: "timeline",
    enabled: editing,
    run: () => void trimToPlayhead("left"),
  },
  {
    id: "trim-right",
    label: t.edit.trimRight,
    shortcuts: [{ key: "w" }],
    scope: "timeline",
    enabled: editing,
    run: () => void trimToPlayhead("right"),
  },
  {
    id: "cut",
    label: t.edit.cut,
    shortcuts: [{ key: "x", ctrl: true }],
    scope: "timeline",
    enabled: clipsSelected,
    run: () => void cutSelection(),
  },
  {
    id: "copy",
    label: t.edit.copy,
    shortcuts: [{ key: "c", ctrl: true }],
    scope: "timeline",
    enabled: clipsSelected,
    run: () => void copySelection(),
  },
  {
    id: "paste",
    label: t.edit.paste,
    // Rust reports an empty clipboard as an error toast.
    shortcuts: [{ key: "v", ctrl: true }],
    scope: "timeline",
    enabled: editing,
    run: () => void pasteAtPlayhead(),
  },
  {
    id: "duplicate",
    label: t.edit.duplicate,
    shortcuts: [{ key: "d", ctrl: true }],
    scope: "timeline",
    enabled: clipsSelected,
    run: () => void duplicateSelection(),
  },
  {
    id: "link-toggle",
    label: t.edit.linkToggle,
    // No shortcut: the design gives none (timeline design §2).
    menuLabel: (ctx) => (ctx.linkToggle === "unlink" ? t.edit.unlink : t.edit.link),
    scope: "timeline",
    enabled: (ctx) => editing(ctx) && ctx.linkToggle !== null,
    run: () => void toggleLinkSelection(),
  },
  {
    id: "detach-audio",
    label: t.edit.detachAudio,
    scope: "timeline",
    enabled: (ctx) => editing(ctx) && ctx.canDetach,
    run: () => void detachSelectedAudio(),
  },
  {
    id: "magnetic",
    label: t.edit.magnetic,
    enabled: editing,
    checked: (ctx) => ctx.magnetic,
    run: () => void toggleMagnetic(),
  },
  {
    id: "preferences",
    label: t.edit.preferences,
    shortcuts: [{ key: ",", ctrl: true }],
    run: () => ui().setPreferencesOpen(true),
  },
  {
    id: "history-panel",
    label: t.view.historyPanel,
    checked: (ctx) => ctx.historyPanelOpen,
    run: () => ui().setHistoryPanelOpen(!ui().historyPanelOpen),
  },
  {
    id: "toggle-media-bin",
    label: t.view.mediaBin,
    checked: (ctx) => ctx.mediaBinOpen,
    run: () => togglePanel("mediaBin"),
  },
  {
    id: "toggle-inspector",
    label: t.view.inspector,
    checked: (ctx) => ctx.inspectorOpen,
    run: () => togglePanel("inspector"),
  },
  {
    id: "reset-panel-layout",
    label: t.view.resetPanelLayout,
    run: resetPanelLayout,
  },
  {
    id: "keyboard-shortcuts",
    label: t.view.keyboardShortcuts,
    shortcuts: [
      { key: "?", shift: "any" },
      { key: "/", ctrl: true, shift: "any" },
    ],
    run: () => ui().setShortcutsOpen(true),
  },
  {
    id: "check-for-updates",
    label: t.help.checkForUpdates,
    run: () => void checkForUpdates({ manual: true }),
  },
  {
    id: "open-logs-folder",
    label: t.help.openLogs,
    run: () => void openLogsFolder(),
  },
  {
    id: "about",
    label: t.help.about,
    run: () => ui().setAboutOpen(true),
  },
  // F1.54, playback design §2. Timeline-scoped, so the arrows keep moving the focus in the media
  // bin's grid and list; a focused slider or menu handles them first (`defaultPrevented`).
  {
    id: "frame-back",
    label: t.view.frameBack,
    shortcuts: [{ key: "ArrowLeft" }],
    scope: "timeline",
    enabled: editing,
    run: () => stepFrames(-1),
  },
  {
    id: "frame-forward",
    label: t.view.frameForward,
    shortcuts: [{ key: "ArrowRight" }],
    scope: "timeline",
    enabled: editing,
    run: () => stepFrames(1),
  },
  {
    id: "frame-back-10",
    label: t.view.frameBack10,
    shortcuts: [{ key: "ArrowLeft", shift: true }],
    scope: "timeline",
    enabled: editing,
    run: () => stepFrames(-10),
  },
  {
    id: "frame-forward-10",
    label: t.view.frameForward10,
    shortcuts: [{ key: "ArrowRight", shift: true }],
    scope: "timeline",
    enabled: editing,
    run: () => stepFrames(10),
  },
  {
    id: "previous-edit",
    label: t.view.previousEdit,
    shortcuts: [{ key: "ArrowUp" }],
    scope: "timeline",
    enabled: editing,
    run: () => jumpToEdit(-1),
  },
  {
    id: "next-edit",
    label: t.view.nextEdit,
    shortcuts: [{ key: "ArrowDown" }],
    scope: "timeline",
    enabled: editing,
    run: () => jumpToEdit(1),
  },
  {
    id: "go-to-start",
    label: t.view.goToStart,
    shortcuts: [{ key: "Home" }],
    scope: "timeline",
    enabled: editing,
    run: goToStart,
  },
  {
    id: "go-to-end",
    label: t.view.goToEnd,
    shortcuts: [{ key: "End" }],
    scope: "timeline",
    enabled: editing,
    run: goToEnd,
  },
  // F1.32. `+` needs Shift on some layouts and `=` is the same key unshifted.
  {
    id: "zoom-in",
    label: t.view.zoomIn,
    shortcuts: [
      { key: "+", shift: "any" },
      { key: "=", shift: "any" },
    ],
    scope: "timeline",
    enabled: editing,
    run: () => zoomStep(1),
  },
  {
    id: "zoom-out",
    label: t.view.zoomOut,
    shortcuts: [{ key: "-" }],
    scope: "timeline",
    enabled: editing,
    run: () => zoomStep(-1),
  },
  {
    id: "zoom-to-fit",
    label: t.view.zoomToFit,
    shortcuts: [{ key: "z", shift: true }],
    scope: "timeline",
    enabled: editing,
    run: fitTimeline,
  },
];

export const SEPARATOR = "-";

export type Menu = {
  id: "file" | "edit" | "view" | "help";
  label: string;
  items: readonly (ActionId | typeof SEPARATOR)[];
};

/** Main menu layout. An action may appear in several menus; it is still registered once. */
export const MENUS: readonly Menu[] = [
  {
    id: "file",
    label: t.menu.file,
    items: [
      "go-home",
      SEPARATOR,
      "new-project",
      "open-project",
      SEPARATOR,
      "save",
      "save-as",
      SEPARATOR,
      "import-media",
      SEPARATOR,
      "project-settings",
      SEPARATOR,
      "exit",
    ],
  },
  {
    id: "edit",
    label: t.menu.edit,
    items: [
      "undo",
      "redo",
      SEPARATOR,
      "cut",
      "copy",
      "paste",
      "duplicate",
      SEPARATOR,
      "split",
      "delete",
      "ripple-delete",
      "trim-left",
      "trim-right",
      SEPARATOR,
      "link-toggle",
      "detach-audio",
      SEPARATOR,
      "magnetic",
      SEPARATOR,
      "select-all",
      "deselect",
      SEPARATOR,
      "history-panel",
      SEPARATOR,
      "preferences",
    ],
  },
  {
    id: "view",
    label: t.menu.view,
    items: [
      "toggle-media-bin",
      "toggle-inspector",
      "history-panel",
      SEPARATOR,
      "zoom-in",
      "zoom-out",
      "zoom-to-fit",
      SEPARATOR,
      "go-to-start",
      "go-to-end",
      "previous-edit",
      "next-edit",
      "frame-back",
      "frame-forward",
      "frame-back-10",
      "frame-forward-10",
      SEPARATOR,
      "reset-panel-layout",
      SEPARATOR,
      "keyboard-shortcuts",
    ],
  },
  {
    id: "help",
    label: t.menu.help,
    items: ["keyboard-shortcuts", SEPARATOR, "check-for-updates", "open-logs-folder", SEPARATOR, "about"],
  },
];

const NO_UI_FLAGS: UiFlags = {
  historyPanelOpen: false,
  mediaBinOpen: false,
  inspectorOpen: false,
  editorOpen: false,
};

const NO_SELECTION: SelectionFlags = { clipsSelected: false, gapSelected: false };

const NO_CLIP_FLAGS: ClipFlags = { linkToggle: null, canDetach: false };

export function actionContext(
  state: ProjectState | null,
  flags: UiFlags = NO_UI_FLAGS,
  selection: SelectionFlags = NO_SELECTION,
  clips: ClipFlags = NO_CLIP_FLAGS,
): ActionContext {
  const history = state?.history;
  return {
    loaded: state !== null,
    canUndo: history?.canUndo ?? false,
    canRedo: history?.canRedo ?? false,
    undoLabel: history?.undoLabel ?? null,
    redoLabel: history?.redoLabel ?? null,
    magnetic: state?.project.magnetic ?? false,
    ...flags,
    ...selection,
    ...clips,
  };
}

type UiStoreState = ReturnType<typeof useUiStore.getState>;

function uiFlags(s: UiStoreState): UiFlags {
  return {
    historyPanelOpen: s.historyPanelOpen,
    mediaBinOpen: !s.collapsedPanels.mediaBin,
    inspectorOpen: !s.collapsedPanels.inspector,
    editorOpen: s.view === "editor",
  };
}

type SelectionStoreState = ReturnType<typeof selectionStore.getState>;

function selectionFlags(s: SelectionStoreState): SelectionFlags {
  return { clipsSelected: s.clips.length > 0, gapSelected: s.gap !== null };
}

export function clipFlags(state: ProjectState | null, selected: readonly ClipId[]): ClipFlags {
  const tracks = state?.project.tracks ?? [];
  return { linkToggle: linkToggle(tracks, selected), canDetach: canDetachAudio(tracks, selected) };
}

/** The current context, for the keyboard handler (outside React). */
export function currentActionContext(): ActionContext {
  const state = useProjectStore.getState().state;
  const selection = selectionStore.getState();
  return actionContext(
    state,
    uiFlags(useUiStore.getState()),
    selectionFlags(selection),
    clipFlags(state, selection.clips),
  );
}

/** The context for components; re-renders only when one of its fields changes. */
export function useActionContext(): ActionContext {
  const flags = useUiStore(useShallow(uiFlags));
  const selection = useStore(selectionStore, useShallow(selectionFlags));
  const selected = useStore(selectionStore, (s) => s.clips);
  // Shallow-compared, so a project change (a drag update) re-renders only when a flag changes.
  const clips = useProjectStore(useShallow((s) => clipFlags(s.state, selected)));
  const project = useProjectStore(useShallow((s) => actionContext(s.state)));
  return { ...project, ...flags, ...selection, ...clips };
}

export function getAction(id: ActionId, actions: readonly Action[] = ACTIONS): Action {
  const action = actions.find((a) => a.id === id);
  if (!action) throw new Error(`unregistered action ${id}`);
  return action;
}

export function isEnabled(action: Action, ctx: ActionContext): boolean {
  return action.enabled?.(ctx) ?? true;
}

export function actionLabel(action: Action, ctx: ActionContext): string {
  return action.menuLabel?.(ctx) ?? action.label;
}

/** Display text of a key combination, e.g. `"Ctrl+Shift+S"`. */
export function bindingLabel(binding: KeyBinding): string {
  const { ctrl, shift, separator, escape, delete: del, left, right, up, down, home, end } = t.keys;
  const named: Record<string, string> = {
    Escape: escape,
    Delete: del,
    ArrowLeft: left,
    ArrowRight: right,
    ArrowUp: up,
    ArrowDown: down,
    Home: home,
    End: end,
  };
  const key = named[binding.key] ?? (binding.key.length === 1 ? binding.key.toUpperCase() : binding.key);
  const parts = [binding.ctrl && ctrl, binding.shift === true && shift, key];
  return parts.filter((part) => typeof part === "string").join(separator);
}

/** Display text of an action's main shortcut; empty when it has none. */
export function shortcutLabel(action: Action): string {
  const first = action.shortcuts?.[0];
  return first ? bindingLabel(first) : "";
}

/** Tooltip for a button that runs an action, e.g. "Undo Add Track (Ctrl+Z)". */
export function actionTooltip(action: Action, ctx: ActionContext): string {
  return t.tooltip(actionLabel(action, ctx), shortcutLabel(action));
}

export type MenuItemView =
  | { kind: "separator" }
  | {
      kind: "action";
      id: ActionId;
      label: string;
      shortcut: string;
      disabled: boolean;
      /** `undefined` for an action that is not a toggle. */
      checked: boolean | undefined;
    };

export type MenuView = { id: Menu["id"]; label: string; items: MenuItemView[] };

/** What the main menu shows for `ctx`. */
export function menuModel(
  ctx: ActionContext,
  menus: readonly Menu[] = MENUS,
  actions: readonly Action[] = ACTIONS,
): MenuView[] {
  return menus.map((menu) => ({
    id: menu.id,
    label: menu.label,
    items: menu.items.map((item): MenuItemView => {
      if (item === SEPARATOR) return { kind: "separator" };
      const action = getAction(item, actions);
      return {
        kind: "action",
        id: action.id,
        label: actionLabel(action, ctx),
        shortcut: shortcutLabel(action),
        disabled: !isEnabled(action, ctx),
        checked: action.checked?.(ctx),
      };
    }),
  }));
}

export type ShortcutGroup = {
  id: Menu["id"];
  label: string;
  rows: { id: ActionId; label: string; keys: string[] }[];
};

/**
 * Every action with a shortcut, grouped by the first menu that lists it, in menu order. Empty
 * groups are left out.
 */
export function shortcutGroups(
  menus: readonly Menu[] = MENUS,
  actions: readonly Action[] = ACTIONS,
): ShortcutGroup[] {
  const seen = new Set<ActionId>();
  const groups: ShortcutGroup[] = [];
  for (const menu of menus) {
    const rows: ShortcutGroup["rows"] = [];
    for (const item of menu.items) {
      if (item === SEPARATOR || seen.has(item)) continue;
      seen.add(item);
      const action = getAction(item, actions);
      if (!action.shortcuts?.length) continue;
      rows.push({ id: action.id, label: action.label, keys: action.shortcuts.map(bindingLabel) });
    }
    if (rows.length > 0) groups.push({ id: menu.id, label: menu.label, rows });
  }
  return groups;
}
