import { describe, expect, it } from "vitest";

import { snap, toFrameGrid } from "./snapping";

describe("snap", () => {
  it("moves the closest candidate onto its target within the threshold", () => {
    expect(snap([100, 300], [0, 105, 296], 10)).toEqual({ delta: -4, target: 296 });
    expect(snap([100], [95, 108], 10)).toEqual({ delta: -5, target: 95 });
  });

  it("does nothing when no target is close enough", () => {
    expect(snap([100], [0, 200], 10)).toEqual({ delta: 0, target: null });
    expect(snap([], [0], 10)).toEqual({ delta: 0, target: null });
  });

  it("includes the threshold itself", () => {
    expect(snap([100], [110], 10)).toEqual({ delta: 10, target: 110 });
  });
});

describe("toFrameGrid", () => {
  it("rounds to the nearest frame and never below zero", () => {
    expect(toFrameGrid(149, 100)).toBe(100);
    expect(toFrameGrid(150, 100)).toBe(200);
    expect(toFrameGrid(-40, 100)).toBe(0);
    expect(toFrameGrid(12.4, 0)).toBe(12);
  });
});
