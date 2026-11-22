import { type Action, ACTIONS, currentActionContext, isEnabled, type KeyBinding } from "./actions";

export type KeyInput = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

export function matchesBinding(input: KeyInput, binding: KeyBinding): boolean {
  const ctrl = input.ctrlKey || input.metaKey;
  return (
    binding.key.toLowerCase() === input.key.toLowerCase() &&
    Boolean(binding.ctrl) === ctrl &&
    (binding.shift === "any" || Boolean(binding.shift) === input.shiftKey) &&
    !input.altKey
  );
}

/** The registered action whose shortcut matches `input`. */
export function findShortcut(input: KeyInput, actions: readonly Action[] = ACTIONS): Action | undefined {
  return actions.find((a) => a.shortcuts?.some((binding) => matchesBinding(input, binding)));
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

/** Runs the action of a matching shortcut. Disabled actions only block the WebView default. */
export function handleShortcut(event: KeyboardEvent, actions: readonly Action[] = ACTIONS): void {
  const action = findShortcut(event, actions);
  if (!action) return;
  const ignored = isTextEntry(event.target) || isInOverlay(event.target);
  if (ignored && !action.blockDefault) return;
  event.preventDefault();
  if (!ignored && isEnabled(action, currentActionContext())) action.run();
}

export function installShortcuts(): () => void {
  const onKeyDown = (event: KeyboardEvent) => handleShortcut(event);
  window.addEventListener("keydown", onKeyDown);
  return () => window.removeEventListener("keydown", onKeyDown);
}
