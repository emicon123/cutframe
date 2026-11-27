/**
 * Preview zoom and pan math (F1.62, render-graph.md §8). Pure: the store and the preview area
 * feed it the zoom, the pan, the preview area's size in physical pixels and the project canvas.
 *
 * Zoom is `fit` or a percentage of canvas pixels per physical screen pixel (100 % = one canvas
 * pixel per screen pixel). Pan is the canvas point shown at the centre of the preview area,
 * relative to the canvas centre, in canvas widths / heights (±0.5 = the edges); `fit` has none.
 */
import {
  PREVIEW_ZOOM_MAX_PERCENT,
  PREVIEW_ZOOM_MIN_PERCENT,
  type PreviewView_Deserialize,
  type PreviewZoom,
} from "@/bindings";

/** The levels of the zoom menu and of the zoom in / out steps. */
export const ZOOM_LEVELS: readonly number[] = [25, 50, 100, 200, 400];

export const FIT: PreviewZoom = { kind: "fit" };

export type Pan = { x: number; y: number };

export const NO_PAN: Pan = { x: 0, y: 0 };

export type Size = { width: number; height: number };

/** Pan limit in canvas fractions from the centre (the engine clamps to the same). */
const PAN_LIMIT = 0.5;

/** Steps ignore a difference below this, so 100 % from a fit of 99.99 % is still a step. */
const EPSILON = 0.5;

export function clampPercent(percent: number): number {
  return Math.round(Math.min(PREVIEW_ZOOM_MAX_PERCENT, Math.max(PREVIEW_ZOOM_MIN_PERCENT, percent)));
}

function clampPan(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(PAN_LIMIT, Math.max(-PAN_LIMIT, value));
}

/**
 * The percentage at which the whole canvas fits `area` (both in pixels; `area` in physical
 * pixels), or `null` while either is unknown or empty.
 */
export function fitPercent(area: Size | null, canvas: Size | null): number | null {
  if (!area || !canvas || area.width <= 0 || area.height <= 0 || canvas.width <= 0 || canvas.height <= 0)
    return null;
  return Math.min(area.width / canvas.width, area.height / canvas.height) * 100;
}

/** The effective percentage of `zoom`: `fit` is the fit percentage (100 while unknown). */
export function effectivePercent(zoom: PreviewZoom, fit: number | null): number {
  return zoom.kind === "percent" ? zoom.percent : (fit ?? 100);
}

/**
 * One zoom in (`1`) or out (`-1`) step from `zoom`: the next level above or below the effective
 * percentage, clamped to the limits.
 */
export function stepZoom(zoom: PreviewZoom, direction: -1 | 1, fit: number | null): PreviewZoom {
  const current = effectivePercent(zoom, fit);
  const next =
    direction > 0
      ? (ZOOM_LEVELS.find((level) => level > current + EPSILON) ?? PREVIEW_ZOOM_MAX_PERCENT)
      : (ZOOM_LEVELS.findLast((level) => level < current - EPSILON) ?? PREVIEW_ZOOM_MIN_PERCENT);
  return { kind: "percent", percent: clampPercent(next) };
}

/** What `setPreviewView` gets: `fit` always centred, a percentage clamped with its pan. */
export function buildView(zoom: PreviewZoom, pan: Pan): PreviewView_Deserialize {
  if (zoom.kind === "fit") return { zoom: FIT, panX: 0, panY: 0 };
  return {
    zoom: { kind: "percent", percent: clampPercent(zoom.percent) },
    panX: clampPan(pan.x),
    panY: clampPan(pan.y),
  };
}

/**
 * The pan after dragging the picture by (`dx`, `dy`) physical pixels at `percent`: the canvas
 * moves with the pointer, so the centre point moves the other way.
 */
export function panBy(pan: Pan, dx: number, dy: number, canvas: Size, percent: number): Pan {
  const scale = percent / 100;
  return {
    x: clampPan(pan.x - dx / (canvas.width * scale)),
    y: clampPan(pan.y - dy / (canvas.height * scale)),
  };
}

/**
 * The pan that keeps the canvas point under `offset` (physical pixels from the preview area's
 * centre) in place when the zoom changes from `from` to `to` percent (Ctrl+wheel).
 */
export function panForZoom(
  pan: Pan,
  offset: { x: number; y: number },
  canvas: Size,
  from: number,
  to: number,
): Pan {
  const point = {
    x: pan.x + offset.x / (canvas.width * (from / 100)),
    y: pan.y + offset.y / (canvas.height * (from / 100)),
  };
  return {
    x: clampPan(point.x - offset.x / (canvas.width * (to / 100))),
    y: clampPan(point.y - offset.y / (canvas.height * (to / 100))),
  };
}
