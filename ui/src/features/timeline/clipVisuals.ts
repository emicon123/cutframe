/**
 * Filmstrips (F1.34) and waveforms (F1.35) on timeline clips: the mapping from a clip's used
 * source range (`sourceIn .. sourceEnd`, mirrored when reversed; clip-speed-audio design §6) onto
 * the `media://` products, and the canvas drawing. Pure and allocation-free on the draw path; no
 * React in here.
 */
import { type ClipSource, FILMSTRIP_FRAME_HEIGHT, type FilmstripLayout, type Time } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import type { Viewport } from "./geometry";

/**
 * A linear map from a timeline offset inside a clip (flicks from its start) to the media time
 * drawn there: `origin + offset × rate`. The clip's width spans its whole used source range, so
 * `rate` is the speed (negative when reversed, 0 for a freeze frame). Display only: Rust owns the
 * exact frame mapping (`Clip::source_time`).
 */
export type SourceMap = { origin: number; rate: number };

/** Reused by `clipSourceMap` on the draw path. */
const sourceMapScratch: SourceMap = { origin: 0, rate: 1 };

/**
 * The source map of a media or freeze clip, written into `out`; `null` for colour and text clips.
 * Media clips map their width linearly onto `sourceIn .. sourceEnd` (from `sourceEnd` down when
 * reversed); a freeze clip shows its `sourceTime` everywhere.
 */
export function clipSourceMap(
  clip: { duration: Time; source: ClipSource; sourceEnd?: Time | null },
  out: SourceMap = sourceMapScratch,
): SourceMap | null {
  const { source } = clip;
  if (source.kind === "freeze") {
    out.origin = source.sourceTime;
    out.rate = 0;
    return out;
  }
  if (source.kind !== "media") return null;
  const end = clip.sourceEnd ?? source.sourceIn + clip.duration;
  const rate = clip.duration > 0 ? (end - source.sourceIn) / clip.duration : 1;
  out.origin = source.reverse ? end : source.sourceIn;
  out.rate = source.reverse ? -rate : rate;
  return out;
}

/** The filmstrip frame that shows media time `time`: frame `i` shows `i × interval`. */
export function filmstripFrame(layout: FilmstripLayout, time: number): number {
  if (layout.frames <= 0 || layout.interval <= 0) return 0;
  return Math.min(layout.frames - 1, Math.max(0, Math.floor(time / layout.interval)));
}

/** Width in CSS px of one filmstrip frame drawn `height` px tall, keeping its aspect. */
export function filmstripSlotWidth(layout: FilmstripLayout, height: number): number {
  return (height * layout.frameWidth) / FILMSTRIP_FRAME_HEIGHT;
}

/** Decoded filmstrip tiles of a media item by tile index; `null` while one is not loaded. */
export type TileSource<Key> = { get(item: Key, tile: number): CanvasImageSource | null };

/**
 * Draws a clip's filmstrip: frame slots side by side from the clip's left edge `x`, each showing
 * the frame at the media time under its middle (through `map`). Only the slots inside `0..width`
 * are drawn; slots whose tile is not loaded yet are left empty (the clip colour shows through).
 */
export function drawFilmstrip<Key>(
  ctx: CanvasRenderingContext2D,
  layout: FilmstripLayout,
  tiles: TileSource<Key>,
  item: Key,
  map: SourceMap,
  x: number,
  w: number,
  y: number,
  h: number,
  viewport: Viewport,
  width: number,
): void {
  const slot = filmstripSlotWidth(layout, h);
  if (slot < 1 || layout.framesPerTile <= 0) return;
  const first = Math.max(0, Math.floor(-x / slot));
  const last = Math.ceil((Math.min(x + w, width) - x) / slot);
  for (let k = first; k < last; k += 1) {
    if (k * slot >= w) break;
    // A partial last slot shows the frame just before the clip's end.
    const offset = Math.min((k + 0.5) * slot, Math.max(0, w - 1));
    const frame = filmstripFrame(layout, map.origin + offset * viewport.flicksPerPixel * map.rate);
    const image = tiles.get(item, Math.floor(frame / layout.framesPerTile));
    if (!image) continue;
    const column = frame % layout.framesPerTile;
    ctx.drawImage(
      image,
      column * layout.frameWidth,
      0,
      layout.frameWidth,
      FILMSTRIP_FRAME_HEIGHT,
      x + k * slot,
      y,
      slot,
      h,
    );
  }
}

/**
 * A waveform with coarser copies for zoomed-out views: `levels[0]` is the `/waveform` body's
 * interleaved (min, max) pairs; each next level merges pairs of buckets of the one before.
 * Built once when the waveform loads, so drawing reads about one bucket per pixel at any zoom.
 */
export type PeakLevels = { bucketsPerSecond: number; levels: Int8Array[] };

export function buildPeakLevels(peaks: Int8Array, bucketsPerSecond: number): PeakLevels {
  const levels = [peaks];
  let current = peaks;
  while (current.length > 2) {
    const buckets = current.length / 2;
    const next = new Int8Array(Math.ceil(buckets / 2) * 2);
    for (let b = 0; b < buckets; b += 2) {
      const pair = b + 1 < buckets;
      const min = current[2 * b] ?? 0;
      const max = current[2 * b + 1] ?? 0;
      next[b] = pair ? Math.min(min, current[2 * b + 2] ?? 0) : min;
      next[b + 1] = pair ? Math.max(max, current[2 * b + 3] ?? 0) : max;
    }
    levels.push(next);
    current = next;
  }
  return { bucketsPerSecond, levels };
}

/** The level to read when one pixel covers `bucketsPerPixel` level-0 buckets. */
export function peakLevel(bucketsPerPixel: number, levelCount: number): number {
  if (bucketsPerPixel <= 2 || levelCount <= 1) return 0;
  return Math.min(levelCount - 1, Math.floor(Math.log2(bucketsPerPixel)));
}

/** The min and max of a bucket range, written into one reused object. */
export const peakScratch = { min: 0, max: 0 };

/**
 * The min and max of level `level`'s buckets covering level-0 buckets `from..to` (at least one
 * bucket); both 0 outside the waveform (silence past its end).
 */
export function readPeak(peaks: PeakLevels, level: number, from: number, to: number, out = peakScratch) {
  const data = peaks.levels[level];
  out.min = 0;
  out.max = 0;
  if (!data) return out;
  const buckets = data.length / 2;
  const start = Math.max(0, Math.floor(from / 2 ** level));
  const end = Math.min(buckets, Math.max(start + 1, Math.ceil(to / 2 ** level)));
  let min = 127;
  let max = -127;
  for (let b = start; b < end; b += 1) {
    min = Math.min(min, data[2 * b] ?? 0);
    max = Math.max(max, data[2 * b + 1] ?? 0);
  }
  if (end > start) {
    out.min = Math.min(min, max);
    out.max = max;
  }
  return out;
}

/**
 * Draws the waveform of a clip's source range (through `map`, so a 2× clip shows twice the audio
 * per pixel and a reversed one shows it mirrored) as centred min/max bars, one per CSS pixel
 * column inside `0..width`, filling `y..y + h` at full scale, in the current fill style.
 */
export function drawClipWaveform(
  ctx: CanvasRenderingContext2D,
  peaks: PeakLevels,
  map: SourceMap,
  x: number,
  w: number,
  y: number,
  h: number,
  viewport: Viewport,
  width: number,
): void {
  const bucketsPerFlick = peaks.bucketsPerSecond / FLICKS_PER_SECOND;
  // Level-0 buckets one column moves through; negative when reversed.
  const step = viewport.flicksPerPixel * map.rate * bucketsPerFlick;
  const perPixel = Math.abs(step);
  const level = peakLevel(perPixel, peaks.levels.length);
  const left = Math.max(0, Math.floor(x));
  const right = Math.min(width, Math.ceil(x + w));
  const mid = y + h / 2;
  const scale = h / 2 / 127;
  // One path and one fill for all bars: far cheaper than a fill per bar.
  ctx.beginPath();
  for (let column = left; column < right; column += 1) {
    const at = (map.origin + (column - x) * viewport.flicksPerPixel * map.rate) * bucketsPerFlick;
    const from = Math.min(at, at + step);
    const peak = readPeak(peaks, level, Math.floor(from), Math.floor(from + perPixel));
    const top = mid - peak.max * scale;
    ctx.rect(column, top, 1, Math.max(1, mid - peak.min * scale - top));
  }
  ctx.fill();
}
