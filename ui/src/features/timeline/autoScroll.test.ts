import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import { createAutoScroll, followPlayhead, USER_HOLD_MS } from "./autoScroll";
import { timeToX, type Viewport } from "./geometry";

const S = FLICKS_PER_SECOND;
// 100 px per second, 1000 px wide: 10 s in view.
const view: Viewport = { scroll: 0, flicksPerPixel: S / 100 };
const WIDTH = 1000;

describe("followPlayhead (F1.31)", () => {
  it("keeps the view while the playhead is inside it", () => {
    expect(followPlayhead(view, WIDTH, 5 * S, 1)).toBe(view);
    expect(followPlayhead(view, WIDTH, 5 * S, -2)).toBe(view);
    expect(followPlayhead(view, WIDTH, 5 * S, 0)).toBe(view);
  });

  it("pages forward when the playhead nears the right edge", () => {
    const next = followPlayhead(view, WIDTH, 9.6 * S, 1);
    expect(timeToX(9.6 * S, next)).toBe(48);
    expect(next.flicksPerPixel).toBe(view.flicksPerPixel);
  });

  it("brings a playhead that left the view back at the edge it comes from", () => {
    const scrolled = { ...view, scroll: 30 * S };
    // Forward (e.g. after a loop wrap to 0): near the left edge, clamped at 0.
    expect(followPlayhead(scrolled, WIDTH, 0, 1).scroll).toBe(0);
    expect(timeToX(12 * S, followPlayhead(scrolled, WIDTH, 12 * S, 2))).toBe(48);
    // Reverse: near the right edge.
    expect(timeToX(29.7 * S, followPlayhead(scrolled, WIDTH, 29.7 * S, -1))).toBe(WIDTH - 48);
    expect(timeToX(50 * S, followPlayhead(scrolled, WIDTH, 50 * S, -1))).toBe(WIDTH - 48);
  });

  it("pages backward in reverse when the playhead nears the left edge", () => {
    const scrolled = { ...view, scroll: 30 * S };
    expect(timeToX(30.2 * S, followPlayhead(scrolled, WIDTH, 30.2 * S, -8))).toBe(WIDTH - 48);
    expect(followPlayhead(scrolled, WIDTH, 30.2 * S, 1)).toBe(scrolled);
  });

  it("keeps the playhead in view over a whole forward run", () => {
    let current = view;
    for (let t = 0; t < 120 * S; t += S / 30) {
      current = followPlayhead(current, WIDTH, t, 1);
      const x = timeToX(t, current);
      expect(x).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThanOrEqual(WIDTH);
    }
  });
});

describe("createAutoScroll", () => {
  it("holds off while the user drags and shortly after a manual scroll", () => {
    const auto = createAutoScroll();
    const far = 50 * S;
    expect(auto.follow(view, WIDTH, far, 1, 0)).not.toBe(view);

    auto.pointerDown();
    expect(auto.follow(view, WIDTH, far, 1, 0)).toBe(view);
    auto.pointerUp();
    expect(auto.follow(view, WIDTH, far, 1, 0)).not.toBe(view);

    auto.scrolled(1000);
    expect(auto.follow(view, WIDTH, far, 1, 1000 + USER_HOLD_MS - 1)).toBe(view);
    expect(auto.follow(view, WIDTH, far, 1, 1000 + USER_HOLD_MS)).not.toBe(view);
  });
});
