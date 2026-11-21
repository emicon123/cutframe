import { commands } from "@/bindings";
import { run } from "@/stores/appStore";

export type KeyInput = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

export type Shortcut = {
  id: string;
  /** `KeyboardEvent.key`, compared case-insensitively. */
  key: string;
  ctrl?: boolean;
  shift?: boolean;
  run: () => void;
};

/** The single registry of editor shortcuts. */
export const SHORTCUTS: readonly Shortcut[] = [
  { id: "undo", key: "z", ctrl: true, run: () => void run(commands.undo()) },
  { id: "redo", key: "z", ctrl: true, shift: true, run: () => void run(commands.redo()) },
  { id: "redo-alt", key: "y", ctrl: true, run: () => void run(commands.redo()) },
];

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

export function installShortcuts(): () => void {
  const onKeyDown = (event: KeyboardEvent) => {
    if (isTextEntry(event.target)) return;
    const shortcut = findShortcut(event);
    if (!shortcut) return;
    event.preventDefault();
    shortcut.run();
  };
  window.addEventListener("keydown", onKeyDown);
  return () => window.removeEventListener("keydown", onKeyDown);
}
