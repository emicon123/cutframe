import type { FrameRate } from "@/bindings";

/** Flicks per second; must match `app_core::FLICKS_PER_SECOND`. */
export const FLICKS_PER_SECOND = 705_600_000;

/** Exact duration of one frame in flicks (all supported rates divide evenly). */
export function frameDuration(rate: FrameRate): number {
  return (FLICKS_PER_SECOND * rate.den) / rate.num;
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
