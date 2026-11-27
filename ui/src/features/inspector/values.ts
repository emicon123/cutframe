/**
 * Conversions between the model's values and what the inspector fields show (clip-speed-audio
 * design §6): volume as percent with a dB readout, speed as a factor on a logarithmic slider,
 * fades and durations in seconds. Pure; the model keeps gains, percent speeds and integer flicks.
 */
import type { Speed, Time } from "@/bindings";
import { MAX_SPEED, MIN_SPEED, NORMAL_SPEED } from "@/features/timeline/retime";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND, secondsToFlicks } from "@/lib/time";

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * A typed number: surrounding spaces, a decimal comma and a trailing unit sign (`%`, `×`, `x`,
 * `s`) are accepted. `null` when it is not a finite number.
 */
export function parseNumber(text: string): number | null {
  const cleaned = text
    .trim()
    .replace(",", ".")
    .replace(/\s*(%|×|x|s)$/i, "");
  if (cleaned === "") return null;
  const value = Number(cleaned);
  return Number.isFinite(value) ? value : null;
}

// ── Volume ────────────────────────────────────────────────────────────────────────────────────

/** The volume field and slider in percent: 0 ..= 400 % (linear gain 0 ..= 4, +12 dB). */
export const MAX_VOLUME_PERCENT = 400;
/** A dragged volume within this many percent of 100 % snaps to 100 % (the detent). */
export const VOLUME_DETENT = 3;

/** Percent typed or dragged → linear gain, clamped to the model's range. */
export function gainFromPercent(percent: number): number {
  return clamp(percent, 0, MAX_VOLUME_PERCENT) / 100;
}

/** The volume slider's position (percent) → gain, with the detent at 100 %. */
export function volumeFromSlider(percent: number): number {
  return gainFromPercent(Math.abs(percent - 100) <= VOLUME_DETENT ? 100 : percent);
}

export function volumeToSlider(gain: number): number {
  return clamp(gain * 100, 0, MAX_VOLUME_PERCENT);
}

/** The volume field's text: whole percent. */
export function formatVolume(gain: number): string {
  return String(Math.round(gain * 100));
}

export function parseVolume(text: string): number | null {
  const percent = parseNumber(text);
  return percent === null ? null : gainFromPercent(percent);
}

/** The dB readout of a gain, `20·log10(v)` with one decimal: "+12.0 dB", "0.0 dB", "−∞ dB". */
export function formatDecibels(gain: number): string {
  if (!(gain > 0)) return t.inspector.audio.silent;
  const db = Math.round(20 * Math.log10(gain) * 10) / 10;
  const sign = db > 0 ? "+" : db < 0 ? "−" : "";
  return t.inspector.audio.decibels(`${sign}${Math.abs(db).toFixed(1)}`);
}

// ── Speed ─────────────────────────────────────────────────────────────────────────────────────

/** The speed presets: 0.25×, 0.5×, 1×, 2×, 4×. */
export const SPEED_PRESETS: readonly Speed[] = [25, 50, 100, 200, 400];

/**
 * A factor (2 = 2×) → `Speed` percent, rounded and clamped to 0.1× ..= 100×. `Speed` fails
 * argument parsing out of range, so every value sent goes through this.
 */
export function speedFromFactor(factor: number): Speed {
  return clamp(Math.round(factor * 100), MIN_SPEED, MAX_SPEED);
}

/** The speed field's text: the factor with two decimals ("2.00"). */
export function formatSpeed(speed: Speed): string {
  return (speed / 100).toFixed(2);
}

export function parseSpeed(text: string): Speed | null {
  const factor = parseNumber(text);
  return factor === null ? null : speedFromFactor(factor);
}

/** The speed slider runs 0 ..= `SPEED_SLIDER_MAX`, logarithmic from 0.1× to 100×. */
export const SPEED_SLIDER_MAX = 1000;
/** A dragged position within this many steps of 1× snaps to 1× (the detent). */
export const SPEED_DETENT = 8;
const SPEED_RANGE = Math.log(MAX_SPEED / MIN_SPEED);

export function speedToSlider(speed: Speed): number {
  const ratio = Math.log(clamp(speed, MIN_SPEED, MAX_SPEED) / MIN_SPEED) / SPEED_RANGE;
  return Math.round(ratio * SPEED_SLIDER_MAX);
}

export function sliderToSpeed(position: number): Speed {
  const at = clamp(position, 0, SPEED_SLIDER_MAX);
  if (Math.abs(at - speedToSlider(NORMAL_SPEED)) <= SPEED_DETENT) return NORMAL_SPEED;
  return clamp(Math.round(MIN_SPEED * Math.exp((at / SPEED_SLIDER_MAX) * SPEED_RANGE)), MIN_SPEED, MAX_SPEED);
}

// ── Fades and durations ───────────────────────────────────────────────────────────────────────

/** The longest fade a clip has room for next to its other fade (Rust clamps the same way). */
export function fadeLimit(duration: Time, otherFade: Time): Time {
  return Math.max(0, duration - otherFade);
}

/** A fade field's text: seconds with one decimal. */
export function formatFade(fade: Time): string {
  return (fade / FLICKS_PER_SECOND).toFixed(1);
}

/** Typed seconds → fade flicks, clamped to `0 ..= limit`. */
export function parseFade(text: string, limit: Time): Time | null {
  const seconds = parseNumber(text);
  return seconds === null ? null : clamp(secondsToFlicks(seconds), 0, Math.max(0, limit));
}

/** A duration field's text: seconds with at most two decimals ("5", "2.5"). */
export function formatDurationSeconds(duration: Time): string {
  return String(Number((duration / FLICKS_PER_SECOND).toFixed(2)));
}

/** Typed seconds → a duration rounded to whole frames of `frame` flicks, at least one frame. */
export function parseDuration(text: string, frame: Time): Time | null {
  const seconds = parseNumber(text);
  if (seconds === null || seconds <= 0) return null;
  return Math.max(1, Math.round(secondsToFlicks(seconds) / frame)) * frame;
}
