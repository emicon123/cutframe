import type { UiLanguage } from "@/bindings";

import { en } from "./en";

/** Translation table per UI language. Polish (F3.74) adds a file and an entry here. */
const LOCALES: Readonly<Record<UiLanguage, typeof en>> = { en };

/** The active locale. Only English exists today; Polish can be added as another file. */
export let t: typeof en = en;

/**
 * Picks the translation table for `language` (the `language` preference). Labels computed at
 * module load (the action registry) keep the table that was active then, so a later switch takes
 * full effect after a restart. With English as the only language this never changes anything.
 */
export function setLocale(language: UiLanguage): void {
  t = Object.hasOwn(LOCALES, language) ? LOCALES[language] : en;
}

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

/** Display text for a `MediaErrorKind`, e.g. `"damaged"` → "the file is damaged". */
export function mediaErrorText(kind: string): string {
  return lookup(t.mediaBin.errors, kind, t.mediaBin.unknownError);
}

/** Display text for a `JobStage`. */
export function jobStageText(stage: string): string {
  return lookup(t.mediaBin.job.stages, stage, t.mediaBin.job.unknownStage);
}
