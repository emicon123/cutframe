import { commands } from "@/bindings";
import { fileActions } from "@/features/project/fileActions";
import { t } from "@/i18n";
import { importFromDialog } from "@/features/media-bin/importActions";
import { run } from "@/stores/appStore";

export type KeyInput = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

export type Shortcut = {
  id: string;
  /** `KeyboardEvent.key`, compared case-insensitively. */
  key: string;
  ctrl?: boolean;
  shift?: boolean;
  /**
   * Always block the WebView2 default (Ctrl+N new window, Ctrl+O/S file dialogs), even where the
   * shortcut itself is ignored (text fields, open dialogs).
   */
  blockDefault?: boolean;
  run: () => void;
};

/** The single registry of editor shortcuts. */
export const SHORTCUTS: readonly Shortcut[] = [
  { id: "undo", key: "z", ctrl: true, run: () => void run(commands.undo()) },
  { id: "redo", key: "z", ctrl: true, shift: true, run: () => void run(commands.redo()) },
  { id: "redo-alt", key: "y", ctrl: true, run: () => void run(commands.redo()) },
  { id: "new-project", key: "n", ctrl: true, blockDefault: true, run: () => void fileActions.newProject() },
  { id: "open-project", key: "o", ctrl: true, blockDefault: true, run: () => void fileActions.openProject() },
  { id: "save", key: "s", ctrl: true, blockDefault: true, run: () => void fileActions.save() },
  {
    id: "save-as",
    key: "s",
    ctrl: true,
    shift: true,
    blockDefault: true,
    run: () => void fileActions.saveAs(),
  },
  { id: "import-media", key: "i", ctrl: true, run: () => void importFromDialog() },
];

/** Display text of a registered shortcut, e.g. `"Ctrl+Shift+S"`; empty for an unknown id. */
export function shortcutLabel(id: string, shortcuts: readonly Shortcut[] = SHORTCUTS): string {
  const shortcut = shortcuts.find((s) => s.id === id);
  if (!shortcut) return "";
  const { ctrl, shift, separator } = t.keys;
  const parts = [shortcut.ctrl && ctrl, shortcut.shift && shift, shortcut.key.toUpperCase()];
  return parts.filter((part) => typeof part === "string").join(separator);
}

export function findShortcut(
  input: KeyInput,
  shortcuts: readonly Shortcut[] = SHORTCUTS,
): Shortcut | undefined {
  const ctrl = input.ctrlKey || input.metaKey;
  return shortcuts.find(
    (s) =>
      s.key.toLowerCase() === input.key.toLowerCase() &&
      Boolean(s.ctrl) === ctrl &&
      Boolean(s.shift) === input.shiftKey &&
      !input.altKey,
  );
}

/** Shortcuts are ignored while the user types. */
export function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA";
}

/**
 * Shortcuts are ignored inside an open menu or dialog (focus is trapped there), so Ctrl+Z or
 * Ctrl+N cannot change the project behind a modal prompt.
 */
export function isInOverlay(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest('[role="dialog"], [role="alertdialog"], [role="menu"]') !== null
  );
}

export function installShortcuts(): () => void {
  const onKeyDown = (event: KeyboardEvent) => {
    const shortcut = findShortcut(event);
    if (!shortcut) return;
    const ignored = isTextEntry(event.target) || isInOverlay(event.target);
    if (ignored && !shortcut.blockDefault) return;
    event.preventDefault();
    if (!ignored) shortcut.run();
  };
  window.addEventListener("keydown", onKeyDown);
  return () => window.removeEventListener("keydown", onKeyDown);
}
