import type { ExportErrorKind, ExportOutcome, IpcError, MediaId } from "@/bindings";
import { formatBytes } from "@/features/preferences/preferences";
import { t } from "@/i18n";
import { errorToast } from "@/ipc/errors";
import { baseName } from "@/lib/fileName";
import type { ToastInput } from "@/stores/toastStore";

/** How an export ended, from its `exportReport` and `jobFinished`. */
export type ExportEnd =
  | { kind: "report"; outcome: ExportOutcome }
  | { kind: "cancelled" }
  /** `jobFinished` failed without a report (should not happen; worded from the `IpcError`). */
  | { kind: "error"; error: IpcError };

/** Failures whose cause the user can see and fix without the log. */
const SELF_EXPLAINING: ReadonlySet<ExportErrorKind> = new Set<ExportErrorKind>(["mediaOffline", "diskFull"]);

/** The reason line of a failed export (export.md §5.3). */
export function failureText(
  outcome: Extract<ExportOutcome, { kind: "failed" }>,
  mediaName: (id: MediaId) => string,
): string {
  const texts = t.export.failures;
  const media = outcome.media ?? null;
  switch (outcome.error) {
    case "mediaOffline":
      return media === null ? texts.mediaOfflineUnknown : texts.mediaOffline(mediaName(media));
    case "decodeFailed":
      return media === null ? texts.decodeFailedUnknown : texts.decodeFailed(mediaName(media));
    case "writeFailed": {
      const kept = outcome.keptPath ?? null;
      return kept === null ? texts.writeFailed : texts.writeFailedKept(kept);
    }
    default:
      return texts[outcome.error];
  }
}

/**
 * The toast for the end of an export: the file name and size when done, the reason when failed
 * (with "Open logs" when the log has more), nothing when cancelled (the card just goes away).
 */
export function exportEndToast(end: ExportEnd, mediaName: (id: MediaId) => string): ToastInput | null {
  switch (end.kind) {
    case "cancelled":
      return null;
    case "error":
      return errorToast(end.error, t.export.failedTitle);
    case "report": {
      const { outcome } = end;
      if (outcome.kind === "done") {
        return {
          tone: "success",
          text: t.export.doneTitle(baseName(outcome.path)),
          detail: formatBytes(outcome.bytes),
        };
      }
      const toast: ToastInput = {
        tone: "error",
        text: t.export.failedTitle,
        detail: failureText(outcome, mediaName),
      };
      if (!SELF_EXPLAINING.has(outcome.error)) toast.action = "openLogs";
      return toast;
    }
  }
}
