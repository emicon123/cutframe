import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  MAX_FLICKS_PER_PIXEL,
  MIN_FLICKS_PER_PIXEL,
  RULER_HEIGHT,
  TRACK_GAP,
  TRACK_HEIGHT,
  formatRulerLabel,
  rulerStep,
  scrollBy,
  timeToX,
  trackAt,
  xToTime,
  zoomAt,
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
