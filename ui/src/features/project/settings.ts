import type { FrameRate, ProjectSettings } from "@/bindings";
import { t } from "@/i18n";

/** Canvas limits; Rust validates the same ranges again (`ProjectSettings::validate`). */
export const MIN_CANVAS_SIZE = 16;
export const MAX_CANVAS_SIZE = 8192;

export type ResolutionPresetId = keyof typeof t.projectSettings.presets;

export const RESOLUTION_PRESETS: readonly {
  id: Exclude<ResolutionPresetId, "custom">;
  width: number;
  height: number;
}[] = [
  { id: "landscapeHd", width: 1920, height: 1080 },
  { id: "landscape4k", width: 3840, height: 2160 },
  { id: "shorts", width: 1080, height: 1920 },
  { id: "square", width: 1080, height: 1080 },
];

/** Offered frame rates, as exact rationals (NTSC rates are x000/1001, never 29.97). */
export const FRAME_RATE_PRESETS: readonly FrameRate[] = [
  { num: 24000, den: 1001 },
  { num: 24, den: 1 },
  { num: 25, den: 1 },
  { num: 30000, den: 1001 },
  { num: 30, den: 1 },
  { num: 50, den: 1 },
  { num: 60, den: 1 },
];

/** The preset matching a canvas size, or `"custom"`. */
export function presetFor(width: number, height: number): ResolutionPresetId {
  return RESOLUTION_PRESETS.find((p) => p.width === width && p.height === height)?.id ?? "custom";
}

/** Stable key for a frame rate, e.g. `"30000/1001"`; used as a `<select>` value. */
export function frameRateKey(rate: FrameRate): string {
  return `${rate.num}/${rate.den}`;
}

/**
 * The frame rates to offer: the presets, plus the project's current rate when it is not one of
 * them (an opened file may use another valid rate), so opening the dialog never changes it.
 */
export function frameRateOptions(current: FrameRate): readonly FrameRate[] {
  const key = frameRateKey(current);
  return FRAME_RATE_PRESETS.some((r) => frameRateKey(r) === key)
    ? FRAME_RATE_PRESETS
    : [...FRAME_RATE_PRESETS, current];
}

export type SizeError = "notInteger" | "odd" | "outOfRange";

export type SizeResult = { ok: true; value: number } | { ok: false; error: SizeError };

/** Parses one canvas dimension typed by the user: a whole, even number in 16..=8192. */
export function parseCanvasSize(text: string): SizeResult {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return { ok: false, error: "notInteger" };
  const value = Number(trimmed);
  if (value < MIN_CANVAS_SIZE || value > MAX_CANVAS_SIZE) return { ok: false, error: "outOfRange" };
  if (value % 2 !== 0) return { ok: false, error: "odd" };
  return { ok: true, value };
}

function gcd(a: number, b: number): number {
  return b === 0 ? a : gcd(b, a % b);
}

/** Derived aspect ratio, reduced by the gcd: 1920×1080 → "16:9", 1080×1920 → "9:16". */
export function formatAspectRatio(width: number, height: number): string {
  const d = gcd(width, height);
  return d === 0 ? "" : `${width / d}:${height / d}`;
}

/** What the settings dialog edits. Dimensions are the raw text of the inputs. */
export type SettingsForm = {
  preset: ResolutionPresetId;
  width: string;
  height: string;
  frameRate: string;
};

export function formFromSettings(settings: ProjectSettings): SettingsForm {
  return {
    preset: presetFor(settings.width, settings.height),
    width: String(settings.width),
    height: String(settings.height),
    frameRate: frameRateKey(settings.frameRate),
  };
}

/** The form after picking a preset: a real preset fills in its size, Custom keeps the current one. */
export function selectPreset(form: SettingsForm, preset: ResolutionPresetId): SettingsForm {
  const found = RESOLUTION_PRESETS.find((p) => p.id === preset);
  return found
    ? { ...form, preset, width: String(found.width), height: String(found.height) }
    : { ...form, preset };
}

export type FormResult =
  { ok: true; settings: ProjectSettings } | { ok: false; width: SizeError | null; height: SizeError | null };

/**
 * Validates the form and builds the settings to send. The frame rate is one of `options`, and the
 * sample rate is sent unchanged from `base`.
 */
export function settingsFromForm(
  form: SettingsForm,
  base: ProjectSettings,
  options: readonly FrameRate[],
): FormResult {
  const width = parseCanvasSize(form.width);
  const height = parseCanvasSize(form.height);
  if (!width.ok || !height.ok) {
    return { ok: false, width: width.ok ? null : width.error, height: height.ok ? null : height.error };
  }
  const frameRate = options.find((r) => frameRateKey(r) === form.frameRate) ?? base.frameRate;
  return {
    ok: true,
    settings: { width: width.value, height: height.value, frameRate, sampleRate: base.sampleRate },
  };
}

export function sizeErrorText(error: SizeError): string {
  const texts = t.projectSettings.sizeErrors;
  return error === "outOfRange" ? texts.outOfRange(MIN_CANVAS_SIZE, MAX_CANVAS_SIZE) : texts[error];
}
