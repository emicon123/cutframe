import type { IpcError } from "@/bindings";
import { errorKindText, t } from "@/i18n";
import type { ToastInput } from "@/stores/toastStore";

export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    typeof value.kind === "string" &&
    "message" in value &&
    typeof value.message === "string"
  );
}

/**
 * User-facing text for anything a command can reject with. Only `IpcError.kind` is translated;
 * diagnostic messages (from Rust, Tauri or JS) are English log text and never shown (ADR-0003 §4-5).
 */
export function describeError(error: unknown): string {
  return isIpcError(error) ? errorKindText(error.kind) : t.errors.unknown;
}

/**
 * Kinds that fully explain themselves (a rejected edit, a user's cancel): the log adds nothing a
 * user can act on, so their toasts carry no "Open logs" button.
 */
const SELF_EXPLAINING_KINDS: ReadonlySet<string> = new Set([
  "invalidEdit",
  "overlap",
  "trackLocked",
  "mediaTooShort",
  "cancelled",
] satisfies IpcError["kind"][]);

/**
 * `true` when the log file holds useful details: Rust logs every failed command with its kind and
 * message (F0.09). Failures that are not `IpcError`s (JS, plugins) only reach the console.
 */
export function hasLoggedDetails(error: unknown): boolean {
  return isIpcError(error) && !SELF_EXPLAINING_KINDS.has(error.kind);
}

/**
 * The toast for a failed command (F0.08). With `title` the toast names what failed and the reason
 * goes on the second line; otherwise the reason is the main line.
 */
export function errorToast(error: unknown, title?: string): ToastInput {
  const reason = describeError(error);
  const logged = hasLoggedDetails(error);
  const toast: ToastInput = title
    ? { tone: "error", text: title, detail: reason }
    : { tone: "error", text: reason };
  if (logged) {
    toast.action = "openLogs";
    if (!title) toast.detail = t.toasts.detailsInLog;
  }
  return toast;
}

/** Writes the diagnostic details of a failure to the console, where developers read them. */
export function logError(error: unknown): void {
  if (isIpcError(error)) console.error(`[ipc] ${error.kind}: ${error.message}`);
  else console.error("[ipc]", error);
}
