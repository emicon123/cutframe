import { en } from "./en";

/** The active locale. Only English exists today; Polish can be added as another file. */
export const t = en;

/**
 * Looks up a backend key in a translation table. Keys arrive at runtime, so one this UI build does
 * not know (or an `Object.prototype` name) gets the fallback instead of the raw key.
 */
function lookup(table: Readonly<Record<string, string>>, key: string, fallback: string): string {
  return Object.hasOwn(table, key) ? (table[key] ?? fallback) : fallback;
}

/** Display text for a `HistoryLabel` key, e.g. `"addTrack"` → "Add Track". */
export function historyLabelText(label: string): string {
  return lookup(t.history.labels, label, t.history.unknown);
}

/** Display text for an `IpcError.kind`. */
export function errorKindText(kind: string): string {
  return lookup(t.errors.kinds, kind, t.errors.unknown);
}
