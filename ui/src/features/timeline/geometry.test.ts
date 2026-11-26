import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  DEFAULT_VIEWPORT,
  FIT_PADDING,
  MAX_FLICKS_PER_PIXEL,
  MIN_FLICKS_PER_PIXEL,
  RULER_HEIGHT,
  TRACK_GAP,
  TRACK_HEIGHT,
  formatRulerLabel,
  revealTime,
  rulerStep,
  scrollBy,
  sliderToZoom,
  timeToX,
  trackAt,
  xToTime,
  zoomAnchor,
  zoomAt,
  zoomTo,
  zoomToFit,
  zoomToSlider,
  ZOOM_SLIDER_STEPS,
  type Viewport,
} from "./geometry";

const viewport: Viewport = { scroll: FLICKS_PER_SECOND, flicksPerPixel: FLICKS_PER_SECOND / 100 };

describe("time <-> x", () => {
  it("round-trips and returns integer flicks", () => {
    const time = 3 * FLICKS_PER_SECOND + 12345;
    const back = xToTime(timeToX(time, viewport), viewport);
    expect(Number.isInteger(back)).toBe(true);
    expect(Math.abs(back - time)).toBeLessThanOrEqual(1);
  });

  it("puts the scroll position at x = 0", () => {
    expect(timeToX(viewport.scroll, viewport)).toBe(0);
  });
});

describe("zoomAt", () => {
  it("keeps the time under the anchor fixed", () => {
    const anchorX = 250;
    const before = xToTime(anchorX, viewport);
    const zoomed = zoomAt(viewport, 0.5, anchorX);
    expect(Math.abs(xToTime(anchorX, zoomed) - before)).toBeLessThanOrEqual(1);
    expect(zoomed.flicksPerPixel).toBe(viewport.flicksPerPixel / 2);
  });

  it("clamps zoom and never scrolls before zero", () => {
    expect(zoomAt(viewport, 1e9, 0).flicksPerPixel).toBe(MAX_FLICKS_PER_PIXEL);
    expect(zoomAt(viewport, 1e-9, 0).flicksPerPixel).toBe(MIN_FLICKS_PER_PIXEL);
    expect(zoomAt({ scroll: 0, flicksPerPixel: viewport.flicksPerPixel }, 2, 400).scroll).toBe(0);
  });
});

describe("scrollBy", () => {
  it("stops at zero", () => {
    expect(scrollBy(viewport, -1e6).scroll).toBe(0);
  });
});

describe("rulerStep", () => {
  it("picks a nice interval at least 90 px wide", () => {
    const step = rulerStep(viewport.flicksPerPixel);
    expect(step / viewport.flicksPerPixel).toBeGreaterThanOrEqual(90);
    expect(step).toBe(FLICKS_PER_SECOND);
  });
});

describe("trackAt", () => {
  it("maps y to track rows, skipping the ruler and gaps", () => {
    expect(trackAt(RULER_HEIGHT - 1, 3)).toBeNull();
    expect(trackAt(RULER_HEIGHT, 3)).toBe(0);
    expect(trackAt(RULER_HEIGHT + TRACK_HEIGHT + 1, 3)).toBeNull();
    expect(trackAt(RULER_HEIGHT + TRACK_HEIGHT + TRACK_GAP, 3)).toBe(1);
    expect(trackAt(RULER_HEIGHT + 3 * (TRACK_HEIGHT + TRACK_GAP), 3)).toBeNull();
  });
});

describe("formatRulerLabel", () => {
  it("formats minutes, hours and sub-second steps", () => {
    expect(formatRulerLabel(5 * FLICKS_PER_SECOND, FLICKS_PER_SECOND)).toBe("0:05");
    expect(formatRulerLabel(3725 * FLICKS_PER_SECOND, FLICKS_PER_SECOND)).toBe("1:02:05");
    expect(formatRulerLabel(2.5 * FLICKS_PER_SECOND, 0.5 * FLICKS_PER_SECOND)).toBe("2.5s");
  });
});

describe("zoom controls (F1.32)", () => {
  it("zooms to a level keeping the anchor time fixed", () => {
    const next = zoomTo(viewport, viewport.flicksPerPixel / 2, 200);
    expect(next.flicksPerPixel).toBe(viewport.flicksPerPixel / 2);
    expect(Math.abs(xToTime(200, next) - xToTime(200, viewport))).toBeLessThanOrEqual(1);
  });

  it("anchors at the playhead while visible, else the middle", () => {
    expect(zoomAnchor(viewport, 1000, viewport.scroll + 300 * viewport.flicksPerPixel)).toBe(300);
    expect(zoomAnchor(viewport, 1000, 0)).toBe(500);
    expect(zoomAnchor(viewport, 1000, viewport.scroll + 2000 * viewport.flicksPerPixel)).toBe(500);
  });

  it("fits the whole timeline into the lanes", () => {
    const duration = 60 * FLICKS_PER_SECOND;
    const fit = zoomToFit(duration, 1024);
    expect(fit.scroll).toBe(0);
    expect(timeToX(duration, fit)).toBeCloseTo(1024 - FIT_PADDING);
  });

  it("fits within the zoom limits and gives the default zoom when empty", () => {
    expect(zoomToFit(0, 1000)).toEqual(DEFAULT_VIEWPORT);
    expect(zoomToFit(FLICKS_PER_SECOND, 0)).toEqual(DEFAULT_VIEWPORT);
    expect(zoomToFit(1, 1000).flicksPerPixel).toBe(MIN_FLICKS_PER_PIXEL);
    expect(zoomToFit(100 * 3600 * FLICKS_PER_SECOND, 1000).flicksPerPixel).toBe(MAX_FLICKS_PER_PIXEL);
  });

  it("maps the slider logarithmically, zoomed in at the right", () => {
    expect(zoomToSlider(MAX_FLICKS_PER_PIXEL)).toBe(0);
    expect(zoomToSlider(MIN_FLICKS_PER_PIXEL)).toBe(ZOOM_SLIDER_STEPS);
    expect(sliderToZoom(0)).toBeCloseTo(MAX_FLICKS_PER_PIXEL);
    expect(sliderToZoom(ZOOM_SLIDER_STEPS)).toBeCloseTo(MIN_FLICKS_PER_PIXEL);
    for (const position of [0, 137, 500, 999]) expect(zoomToSlider(sliderToZoom(position))).toBe(position);
    const middle = sliderToZoom(ZOOM_SLIDER_STEPS / 2);
    expect(middle).toBeCloseTo(Math.sqrt(MIN_FLICKS_PER_PIXEL * MAX_FLICKS_PER_PIXEL));
    expect(sliderToZoom(-5)).toBeCloseTo(MAX_FLICKS_PER_PIXEL);
  });
});

describe("revealTime", () => {
  it("leaves a time in view alone", () => {
    const inView = viewport.scroll + 500 * viewport.flicksPerPixel;
    expect(revealTime(viewport, 1000, inView)).toBe(viewport);
  });

  it("scrolls a time past either edge just into view", () => {
    const right = revealTime(viewport, 1000, viewport.scroll + 1500 * viewport.flicksPerPixel);
    expect(timeToX(viewport.scroll + 1500 * viewport.flicksPerPixel, right)).toBeCloseTo(952);
    const left = revealTime(viewport, 1000, viewport.scroll - 10 * viewport.flicksPerPixel);
    expect(timeToX(viewport.scroll - 10 * viewport.flicksPerPixel, left)).toBeCloseTo(48);
    expect(revealTime(viewport, 1000, 0).scroll).toBe(0);
  });
});
