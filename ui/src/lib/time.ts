import type { FrameRate } from "@/bindings";

/** Flicks per second; must match `app_core::FLICKS_PER_SECOND`. */
export const FLICKS_PER_SECOND = 705_600_000;

/** Exact duration of one frame in flicks (all supported rates divide evenly). */
export function frameDuration(rate: FrameRate): number {
  return (FLICKS_PER_SECOND * rate.den) / rate.num;
}

/** Seconds typed by the user → flicks, rounded to the nearest flick (time is never stored as seconds). */
export function secondsToFlicks(seconds: number): number {
  return Math.round(seconds * FLICKS_PER_SECOND);
}

/** Display only. Never store the result. */
export function flicksToSeconds(flicks: number): number {
  return flicks / FLICKS_PER_SECOND;
}

/** Non-drop-frame timecode `HH:MM:SS:FF` (NTSC rates use their nominal frame count, e.g. 30). */
export function formatTimecode(flicks: number, rate: FrameRate): string {
  const nominalFps = Math.round(rate.num / rate.den);
  const totalFrames = Math.floor(Math.max(0, flicks) / frameDuration(rate));
  const frames = totalFrames % nominalFps;
  const totalSeconds = Math.floor(totalFrames / nominalFps);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(totalSeconds / 3600))}:${pad(Math.floor(totalSeconds / 60) % 60)}:${pad(totalSeconds % 60)}:${pad(frames)}`;
}

export function formatFrameRate(rate: FrameRate): string {
  const fps = rate.num / rate.den;
  return Number.isInteger(fps) ? `${fps} fps` : `${fps.toFixed(3)} fps`;
}

/** A media length for lists: `M:SS`, or `H:MM:SS` from one hour, rounded to the nearest second. */
export function formatDuration(flicks: number): string {
  const totalSeconds = Math.round(Math.max(0, flicks) / FLICKS_PER_SECOND);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

/**
 * A clip's length on its timeline label: tenths of a second below one minute ("4.2s"), otherwise
 * like `formatDuration` ("1:05"). Truncated, not rounded, so a label never claims more than is there.
 */
export function formatClipDuration(flicks: number): string {
  const tenths = Math.floor((Math.max(0, flicks) * 10) / FLICKS_PER_SECOND);
  if (tenths < 600) return `${(tenths / 10).toFixed(1)}s`;
  return formatDuration(Math.floor(tenths / 10) * FLICKS_PER_SECOND);
}
