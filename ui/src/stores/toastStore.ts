import { create } from "zustand";

export type ToastTone = "info" | "success" | "error";

/** A button on a toast. Only one exists today: open the logs folder (F0.09). */
export type ToastAction = "openLogs";

export type Toast = {
  id: number;
  tone: ToastTone;
  /** Translated main line. */
  text: string;
  /** Translated second line, e.g. the reason or "Details are in the log file." */
  detail?: string;
  action?: ToastAction;
  /** How many times the same message arrived while it was shown (shown as "×3"). */
  count: number;
};

export type ToastInput = Omit<Toast, "id" | "count">;

/** Info and success go quickly; errors stay longer so they can be read (F0.08). */
export const TOAST_DURATION_MS: Readonly<Record<ToastTone, number>> = {
  info: 4000,
  success: 4000,
  error: 10000,
};

/** More than this and the oldest toast is dropped. */
export const MAX_TOASTS = 4;

function sameMessage(a: ToastInput, b: ToastInput): boolean {
  return a.tone === b.tone && a.text === b.text && a.detail === b.detail;
}

/**
 * The queue after `input` arrives with `id`. A message that is already shown moves to the newest
 * place with a new id (restarting its timer) and a higher count instead of stacking a copy, so a
 * repeated failure (Ctrl+V on an empty clipboard) cannot flood the screen.
 */
export function addToast(toasts: readonly Toast[], input: ToastInput, id: number): Toast[] {
  const previous = toasts.find((toast) => sameMessage(toast, input));
  const rest = previous ? toasts.filter((toast) => toast !== previous) : toasts;
  const next: Toast = { ...input, id, count: (previous?.count ?? 0) + 1 };
  return [...rest, next].slice(-MAX_TOASTS);
}

export function removeToast(toasts: readonly Toast[], id: number): Toast[] {
  return toasts.filter((toast) => toast.id !== id);
}

type ToastStore = {
  /** Oldest first. */
  toasts: Toast[];
  nextId: number;
  show: (input: ToastInput) => void;
  dismiss: (id: number) => void;
};

/** Non-blocking notifications, rendered by `Toaster`. */
export const useToastStore = create<ToastStore>()((set) => ({
  toasts: [],
  nextId: 1,
  show: (input) => set((s) => ({ toasts: addToast(s.toasts, input, s.nextId), nextId: s.nextId + 1 })),
  dismiss: (id) => set((s) => ({ toasts: removeToast(s.toasts, id) })),
}));

/** Shows a toast from anywhere (outside React too). */
export function showToast(input: ToastInput): void {
  useToastStore.getState().show(input);
}
