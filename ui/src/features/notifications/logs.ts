import { commands } from "@/bindings";
import { t } from "@/i18n";
import { describeError, isIpcError, logError } from "@/ipc/errors";
import { showToast, type ToastInput } from "@/stores/toastStore";

/**
 * The toast for a failed `openLogsFolder`. It has no "Open logs" button: that would only fail
 * again. `unavailable` means the app logs to the console only.
 */
export function openLogsFailedToast(error: unknown): ToastInput {
  if (isIpcError(error) && error.kind === "unavailable") return { tone: "error", text: t.logs.unavailable };
  return { tone: "error", text: t.logs.openFailed, detail: describeError(error) };
}

/** Help → Open logs folder, and the "Open logs" button on error toasts and dialogs (F0.09). */
export async function openLogsFolder(): Promise<void> {
  try {
    await commands.openLogsFolder();
  } catch (error) {
    logError(error);
    showToast(openLogsFailedToast(error));
  }
}
