import { create } from "zustand";

import { t } from "@/i18n";
import { describeError, hasLoggedDetails } from "@/ipc/errors";

/** The flows whose failures need a decision rather than a toast. */
export type FailureContext = "open" | "save";

export type ErrorDialogContent = {
  title: string;
  /** The translated reason, from `IpcError.kind`. */
  reason: string;
  /** What the user can do next; absent when there is nothing beyond closing. */
  hint?: string;
  /** Offer "Open logs folder": Rust logged the details of this failure. */
  logs: boolean;
  /** Offer Save As… (a failed save: the changes are still only in memory). */
  saveAs: boolean;
};

/** What the error dialog shows for a failure in `context`. */
export function errorDialogContent(context: FailureContext, error: unknown): ErrorDialogContent {
  const base = { reason: describeError(error), logs: hasLoggedDetails(error) };
  return context === "save"
    ? { ...base, title: t.errorDialog.saveTitle, hint: t.errorDialog.saveHint, saveAs: true }
    : { ...base, title: t.errorDialog.openTitle, saveAs: false };
}

type ErrorDialogStore = { content: ErrorDialogContent | null };

/** The open error dialog, rendered by `ErrorDialog`. A new failure replaces the one shown. */
export const useErrorDialog = create<ErrorDialogStore>()(() => ({ content: null }));

export function showErrorDialog(context: FailureContext, error: unknown): void {
  useErrorDialog.setState({ content: errorDialogContent(context, error) });
}

export function closeErrorDialog(): void {
  useErrorDialog.setState({ content: null });
}
