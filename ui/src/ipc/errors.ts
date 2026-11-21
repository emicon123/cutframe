import type { IpcError } from "@/bindings";
import { errorKindText, t } from "@/i18n";

function isIpcError(value: unknown): value is IpcError {
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

/** Writes the diagnostic details of a failure to the console, where developers read them. */
export function logError(error: unknown): void {
  if (isIpcError(error)) console.error(`[ipc] ${error.kind}: ${error.message}`);
  else console.error("[ipc]", error);
}
