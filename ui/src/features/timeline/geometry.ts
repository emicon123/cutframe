/**
 * Pure timeline geometry. Time is always in flicks (integers); pixels are CSS pixels relative to
 * the left edge of the lanes.
 */
import { FLICKS_PER_SECOND } from "@/lib/time";

export const RULER_HEIGHT = 28;
export const TRACK_HEIGHT = 56;
export const TRACK_GAP = 2;

/** Zoom limits: from ~1 frame per 20 px to ~2 hours across 1500 px. */
export const MIN_FLICKS_PER_PIXEL = FLICKS_PER_SECOND / 1200;
export const MAX_FLICKS_PER_PIXEL = (FLICKS_PER_SECOND * 7200) / 1500;

export type Viewport = {
  /** Time at the left edge of the lanes. */
  scroll: number;
  /** Zoom level. */
  flicksPerPixel: number;
};

export const DEFAULT_VIEWPORT: Viewport = { scroll: 0, flicksPerPixel: FLICKS_PER_SECOND / 80 };

export function timeToX(time: number, viewport: Viewport): number {
  return (time - viewport.scroll) / viewport.flicksPerPixel;
}

/** Rounded to whole flicks so results can go back into the model. */
export function xToTime(x: number, viewport: Viewport): number {
  return Math.round(viewport.scroll + x * viewport.flicksPerPixel);
}

/** Zooms by `factor` (> 1 zooms out) keeping the time under `anchorX` fixed. */
export function zoomAt(viewport: Viewport, factor: number, anchorX: number): Viewport {
  const anchorTime = viewport.scroll + anchorX * viewport.flicksPerPixel;
  const flicksPerPixel = Math.min(
    MAX_FLICKS_PER_PIXEL,
    Math.max(MIN_FLICKS_PER_PIXEL, viewport.flicksPerPixel * factor),
  );
  return { flicksPerPixel, scroll: Math.max(0, Math.round(anchorTime - anchorX * flicksPerPixel)) };
}

export function scrollBy(viewport: Viewport, deltaX: number): Viewport {
  return { ...viewport, scroll: Math.max(0, Math.round(viewport.scroll + deltaX * viewport.flicksPerPixel)) };
}

const NICE_STEPS_SECONDS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/** Spacing of labelled ruler ticks: the smallest "nice" interval at least `minPixels` wide. */
export function rulerStep(flicksPerPixel: number, minPixels = 90): number {
  for (const seconds of NICE_STEPS_SECONDS) {
    const flicks = seconds * FLICKS_PER_SECOND;
    if (flicks / flicksPerPixel >= minPixels) return flicks;
  }
  return 7200 * FLICKS_PER_SECOND;
}

/** Top of track `index` below the ruler. */
export function trackTop(index: number): number {
  return RULER_HEIGHT + index * (TRACK_HEIGHT + TRACK_GAP);
}

/** Index of the track under `y`, or `null` for the ruler, gaps and empty space. */
export function trackAt(y: number, trackCount: number): number | null {
  if (y < RULER_HEIGHT) return null;
  const index = Math.floor((y - RULER_HEIGHT) / (TRACK_HEIGHT + TRACK_GAP));
  const withinTrack = (y - RULER_HEIGHT) % (TRACK_HEIGHT + TRACK_GAP) < TRACK_HEIGHT;
  return index < trackCount && withinTrack ? index : null;
}

/** Ruler label, e.g. "0:05", "1:30", "1:00:00", or "2.5s" below one second steps. */
export function formatRulerLabel(time: number, step: number): string {
  const seconds = time / FLICKS_PER_SECOND;
  if (step < FLICKS_PER_SECOND) return `${seconds.toFixed(1)}s`;
  const whole = Math.round(seconds);
  const h = Math.floor(whole / 3600);
  const m = Math.floor(whole / 60) % 60;
  const s = String(whole % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Sets the zoom to `flicksPerPixel` (clamped) keeping the time under `anchorX` fixed. */
export function zoomTo(viewport: Viewport, flicksPerPixel: number, anchorX: number): Viewport {
  return zoomAt(viewport, flicksPerPixel / viewport.flicksPerPixel, anchorX);
}

/**
 * Where keyboard and slider zooms are anchored (F1.32): at the playhead while it is in view, so
 * it stays put, otherwise at the middle of the lanes.
 */
export function zoomAnchor(viewport: Viewport, width: number, playhead: number): number {
  const x = timeToX(playhead, viewport);
  return x >= 0 && x <= width ? x : width / 2;
}

/** Empty space left on each side of the content by zoom to fit, in CSS px. */
export const FIT_PADDING = 24;

/**
 * Zoom to fit (F1.32): the whole timeline (0 to `duration`) fills `width` with `FIT_PADDING` to
 * spare at the right. An empty timeline gets the default zoom; both scroll to 0.
 */
export function zoomToFit(duration: number, width: number): Viewport {
  if (duration <= 0 || width <= FIT_PADDING * 2) return { ...DEFAULT_VIEWPORT };
  const flicksPerPixel = Math.min(
    MAX_FLICKS_PER_PIXEL,
    Math.max(MIN_FLICKS_PER_PIXEL, duration / (width - FIT_PADDING)),
  );
  return { scroll: 0, flicksPerPixel };
}

/** Slider positions run from 0 (zoomed out the most) to `ZOOM_SLIDER_STEPS` (zoomed in the most). */
export const ZOOM_SLIDER_STEPS = 1000;

const ZOOM_RANGE = Math.log(MAX_FLICKS_PER_PIXEL / MIN_FLICKS_PER_PIXEL);

/** The slider position of a zoom level; logarithmic, so each step zooms by the same factor. */
export function zoomToSlider(flicksPerPixel: number): number {
  const position = (Math.log(MAX_FLICKS_PER_PIXEL / flicksPerPixel) / ZOOM_RANGE) * ZOOM_SLIDER_STEPS;
  return Math.min(ZOOM_SLIDER_STEPS, Math.max(0, Math.round(position)));
}

/** The zoom level of a slider position (the inverse of `zoomToSlider`). */
export function sliderToZoom(position: number): number {
  const clamped = Math.min(ZOOM_SLIDER_STEPS, Math.max(0, position));
  return MAX_FLICKS_PER_PIXEL * Math.exp((-clamped / ZOOM_SLIDER_STEPS) * ZOOM_RANGE);
}

/** One zoom-in or zoom-out key press (+ / -) or toolbar click. */
export const ZOOM_STEP = 1.5;

/**
 * The viewport scrolled just enough that `time` is in view, `margin` px from the nearer edge;
 * unchanged when it already is. For keyboard seeks (F1.54) that land off screen.
 */
export function revealTime(viewport: Viewport, width: number, time: number, margin = 48): Viewport {
  const x = timeToX(time, viewport);
  const inset = Math.min(margin, width / 4);
  if (x >= inset && x <= width - inset) return viewport;
  const target = x < inset ? inset : width - inset;
  return { ...viewport, scroll: Math.max(0, Math.round(time - target * viewport.flicksPerPixel)) };
}
