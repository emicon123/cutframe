import { describe, expect, it } from "vitest";

import { PREVIEW_ZOOM_MAX_PERCENT, PREVIEW_ZOOM_MIN_PERCENT } from "@/bindings";

import {
  buildView,
  clampPercent,
  effectivePercent,
  FIT,
  fitPercent,
  NO_PAN,
  panBy,
  panForZoom,
  stepZoom,
} from "./previewView";

const canvas = { width: 1920, height: 1080 };
const percent = (p: number) => ({ kind: "percent" as const, percent: p });

describe("fitPercent", () => {
  it("is the scale at which the whole canvas fits the area", () => {
    expect(fitPercent({ width: 960, height: 1080 }, canvas)).toBe(50);
    expect(fitPercent({ width: 3840, height: 1080 }, canvas)).toBe(100);
  });

  it("is unknown for an empty area or canvas", () => {
    expect(fitPercent(null, canvas)).toBeNull();
    expect(fitPercent({ width: 0, height: 100 }, canvas)).toBeNull();
    expect(fitPercent({ width: 100, height: 100 }, { width: 0, height: 0 })).toBeNull();
  });
});

describe("stepZoom", () => {
  it("steps from fit to the next level above or below the fit scale", () => {
    expect(stepZoom(FIT, 1, 66.7)).toEqual(percent(100));
    expect(stepZoom(FIT, -1, 66.7)).toEqual(percent(50));
    expect(effectivePercent(FIT, null)).toBe(100);
    expect(stepZoom(FIT, 1, null)).toEqual(percent(200));
  });

  it("steps between levels and skips a level it is already on", () => {
    expect(stepZoom(percent(100), 1, 50)).toEqual(percent(200));
    expect(stepZoom(percent(100), -1, 50)).toEqual(percent(50));
    expect(stepZoom(FIT, 1, 99.9)).toEqual(percent(200));
  });

  it("stays inside the engine's limits", () => {
    expect(stepZoom(percent(PREVIEW_ZOOM_MAX_PERCENT), 1, 50)).toEqual(percent(PREVIEW_ZOOM_MAX_PERCENT));
    expect(stepZoom(percent(PREVIEW_ZOOM_MIN_PERCENT), -1, 50)).toEqual(percent(PREVIEW_ZOOM_MIN_PERCENT));
    expect(stepZoom(FIT, -1, 10)).toEqual(percent(PREVIEW_ZOOM_MIN_PERCENT));
    expect(clampPercent(1000)).toBe(PREVIEW_ZOOM_MAX_PERCENT);
    expect(clampPercent(3)).toBe(PREVIEW_ZOOM_MIN_PERCENT);
    expect(clampPercent(150.4)).toBe(150);
  });
});

describe("buildView", () => {
  it("ignores the pan at fit", () => {
    expect(buildView(FIT, { x: 0.3, y: -0.2 })).toEqual({ zoom: FIT, panX: 0, panY: 0 });
  });

  it("clamps the percentage and the pan", () => {
    expect(buildView(percent(800), { x: 0.9, y: -0.7 })).toEqual({
      zoom: percent(PREVIEW_ZOOM_MAX_PERCENT),
      panX: 0.5,
      panY: -0.5,
    });
    expect(buildView(percent(200), { x: 0.1, y: Number.NaN })).toEqual({
      zoom: percent(200),
      panX: 0.1,
      panY: 0,
    });
  });
});

describe("panBy", () => {
  it("moves the picture with the pointer, so the centre moves the other way", () => {
    // At 100 %, dragging 192 px right shows a point 10 % of the canvas width further left.
    const pan = panBy(NO_PAN, 192, -108, canvas, 100);
    expect(pan.x).toBeCloseTo(-0.1);
    expect(pan.y).toBeCloseTo(0.1);
    // At 200 % the same drag moves half as far through the canvas.
    expect(panBy(NO_PAN, 192, 0, canvas, 200).x).toBeCloseTo(-0.05);
    expect(panBy(NO_PAN, -100_000, 0, canvas, 100).x).toBe(0.5);
  });
});

describe("panForZoom", () => {
  it("keeps the point under the pointer in place", () => {
    const offset = { x: 480, y: 0 };
    const pan = panForZoom(NO_PAN, offset, canvas, 50, 100);
    // At 50 % the pointer is over the canvas point 0.5 widths right of the centre: the edge.
    // At 100 % that point stays under the pointer, 480 px right of the area's centre.
    expect(pan.x + offset.x / canvas.width).toBeCloseTo(0.5);
    expect(pan.y).toBe(0);
    expect(panForZoom(NO_PAN, { x: 0, y: 0 }, canvas, 50, 200)).toEqual({ x: 0, y: 0 });
  });
});
