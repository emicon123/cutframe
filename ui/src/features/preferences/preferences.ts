import type { IpcError, Preferences_Serialize, Time, UiLanguage } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND, secondsToFlicks } from "@/lib/time";

export { secondsToFlicks };

/** The preferences as Rust returns them (every field present). */
export type Preferences = Preferences_Serialize;

/** Flicks → the text shown in a seconds field, with at most three decimals ("5", "0.5"). */
export function flicksToSecondsText(flicks: Time): string {
  return String(Number((flicks / FLICKS_PER_SECOND).toFixed(3)));
}

/** A seconds field's allowed range; must match `src-tauri/src/preferences.rs`. */
export type SecondsRange = { min: number; max: number; whole: boolean };

export const AUTOSAVE_RANGE: SecondsRange = { min: 15, max: 3600, whole: true };
export const IMAGE_DURATION_RANGE: SecondsRange = { min: 0.1, max: 600, whole: false };
export const TRANSITION_DURATION_RANGE: SecondsRange = { min: 0.1, max: 10, whole: false };
/** Value the autosave field shows when autosave is switched back on. */
export const DEFAULT_AUTOSAVE_SECONDS = 60;

export type SecondsError = "notNumber" | "notWhole" | "outOfRange";
export type ParsedSeconds = { ok: true; flicks: Time } | { ok: false; error: SecondsError };

/** Parses a seconds field and checks it against `range`. */
export function parseSeconds(text: string, range: SecondsRange): ParsedSeconds {
  const trimmed = text.trim();
  const seconds = trimmed === "" ? Number.NaN : Number(trimmed);
  if (!Number.isFinite(seconds)) return { ok: false, error: "notNumber" };
  if (range.whole && !Number.isInteger(seconds)) return { ok: false, error: "notWhole" };
  const flicks = secondsToFlicks(seconds);
  if (flicks < secondsToFlicks(range.min) || flicks > secondsToFlicks(range.max)) {
    return { ok: false, error: "outOfRange" };
  }
  return { ok: true, flicks };
}

export function secondsErrorText(error: SecondsError, range: SecondsRange): string {
  if (error === "outOfRange")
    return t.preferences.fieldErrors.outOfRange(String(range.min), String(range.max));
  return t.preferences.fieldErrors[error];
}

/** The fields of the dialog that Save sends. The cache folder changes at once, not via Save. */
export type PreferencesForm = {
  language: UiLanguage;
  autosaveEnabled: boolean;
  autosaveSeconds: string;
  imageSeconds: string;
  transitionSeconds: string;
};

export type FormField = "autosaveSeconds" | "imageSeconds" | "transitionSeconds";

export function formFromPreferences(prefs: Preferences): PreferencesForm {
  return {
    language: prefs.language,
    autosaveEnabled: prefs.autosaveInterval !== null,
    autosaveSeconds:
      prefs.autosaveInterval === null
        ? String(DEFAULT_AUTOSAVE_SECONDS)
        : flicksToSecondsText(prefs.autosaveInterval),
    imageSeconds: flicksToSecondsText(prefs.defaultImageDuration),
    transitionSeconds: flicksToSecondsText(prefs.defaultTransitionDuration),
  };
}

export type FieldErrors = Partial<Record<FormField, SecondsError>>;

/** Per-field validation hints; empty when the form can be saved. Rust validates again. */
export function validateForm(form: PreferencesForm): FieldErrors {
  const errors: FieldErrors = {};
  const check = (field: FormField, range: SecondsRange) => {
    const parsed = parseSeconds(form[field], range);
    if (!parsed.ok) errors[field] = parsed.error;
  };
  if (form.autosaveEnabled) check("autosaveSeconds", AUTOSAVE_RANGE);
  check("imageSeconds", IMAGE_DURATION_RANGE);
  check("transitionSeconds", TRANSITION_DURATION_RANGE);
  return errors;
}

/**
 * The full object to send to `setPreferences`: `base` (what Rust returned last, so fields this
 * build does not edit are kept) with the form's values. A field whose text is unchanged keeps its
 * exact flicks, so saving without edits never rounds a stored value. `null` when invalid.
 */
export function preferencesFromForm(
  form: PreferencesForm,
  base: Preferences,
  initial: PreferencesForm,
): Preferences | null {
  if (Object.keys(validateForm(form)).length > 0) return null;
  const value = (field: FormField, range: SecondsRange, original: Time): Time => {
    if (form[field] === initial[field]) return original;
    const parsed = parseSeconds(form[field], range);
    return parsed.ok ? parsed.flicks : original;
  };
  const autosave = (): Time | null => {
    if (!form.autosaveEnabled) return null;
    if (base.autosaveInterval !== null && form.autosaveSeconds === initial.autosaveSeconds) {
      return base.autosaveInterval;
    }
    const parsed = parseSeconds(form.autosaveSeconds, AUTOSAVE_RANGE);
    return parsed.ok ? parsed.flicks : null;
  };
  return {
    ...base,
    language: form.language,
    autosaveInterval: autosave(),
    defaultImageDuration: value("imageSeconds", IMAGE_DURATION_RANGE, base.defaultImageDuration),
    defaultTransitionDuration: value(
      "transitionSeconds",
      TRANSITION_DURATION_RANGE,
      base.defaultTransitionDuration,
    ),
  };
}

const FIELD_BY_NAME: Readonly<Record<string, FormField>> = {
  autosaveInterval: "autosaveSeconds",
  defaultImageDuration: "imageSeconds",
  defaultTransitionDuration: "transitionSeconds",
};

/**
 * The form field an `invalidEdit` from `setPreferences` is about. Rust's diagnostic message starts
 * with the camelCase field name; it is only parsed here, never shown (ADR-0003 §5).
 */
export function errorField(error: unknown): FormField | null {
  if (typeof error !== "object" || error === null) return null;
  const { kind, message } = error as Partial<IpcError>;
  if (kind !== "invalidEdit" || typeof message !== "string") return null;
  const name = /^[A-Za-z]+/.exec(message)?.[0];
  return name !== undefined && Object.hasOwn(FIELD_BY_NAME, name) ? (FIELD_BY_NAME[name] ?? null) : null;
}

const UNITS = ["KB", "MB", "GB", "TB"] as const;

/**
 * A byte count from `CacheInfo.sizeBytes` (a decimal string, 64-bit) for display, 1024-based
 * like Explorer, e.g. "1.5 MB". Computed with `BigInt`, so sizes above 2^53 stay exact; the shown
 * decimal is truncated, never rounded up to the next unit.
 */
export function formatBytes(sizeBytes: string): string {
  let bytes: bigint;
  try {
    bytes = BigInt(sizeBytes.trim());
  } catch {
    return t.preferences.cache.sizeUnknown;
  }
  if (sizeBytes.trim() === "" || bytes < 0n) return t.preferences.cache.sizeUnknown;
  if (bytes < 1024n) return t.units.bytes(bytes.toString());
  let divisor = 1024n;
  let unit = 0;
  while (unit < UNITS.length - 1 && bytes >= divisor * 1024n) {
    divisor *= 1024n;
    unit += 1;
  }
  const tenths = (bytes * 10n) / divisor;
  const value = `${(tenths / 10n).toString()}.${(tenths % 10n).toString()}`;
  return t.units.scaled(value, UNITS[unit] ?? "TB");
}
