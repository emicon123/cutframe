import { describe, expect, it } from "vitest";

import { frameDuration } from "@/lib/time";

import { editPoints, frameStep, lastFrame, nextEdit, previousEdit } from "./navigation";

const frame = frameDuration({ num: 30, den: 1 });
const ntsc = frameDuration({ num: 30000, den: 1001 });

describe("frameStep", () => {
  it("steps whole frames on the grid", () => {
    expect(frameStep(0, 1, frame, 100 * frame)).toBe(frame);
    expect(frameStep(5 * frame, -1, frame, 100 * frame)).toBe(4 * frame);
    expect(frameStep(5 * frame, 10, frame, 100 * frame)).toBe(15 * frame);
    expect(frameStep(15 * frame, -10, frame, 100 * frame)).toBe(5 * frame);
  });

  it("snaps a playhead between frames to the neighbour in the step direction first", () => {
    const between = 5 * frame + 123;
    expect(frameStep(between, 1, frame, 100 * frame)).toBe(6 * frame);
    expect(frameStep(between, -1, frame, 100 * frame)).toBe(5 * frame);
  });

  it("stays on the NTSC grid, as integers", () => {
    const time = frameStep(0, 1001, ntsc, 10_000 * ntsc);
    expect(Number.isInteger(time)).toBe(true);
    expect(time).toBe(1001 * ntsc);
  });

  it("clamps to 0 and the end", () => {
    expect(frameStep(0, -1, frame, 100 * frame)).toBe(0);
    expect(frameStep(2 * frame, -10, frame, 100 * frame)).toBe(0);
    expect(frameStep(99 * frame, 10, frame, 100 * frame)).toBe(100 * frame);
    expect(frameStep(0, 1, frame, 0)).toBe(0);
  });
});

describe("lastFrame", () => {
  it("is the start of the last frame, or 0 for an empty timeline", () => {
    expect(lastFrame(100 * frame, frame)).toBe(99 * frame);
    expect(lastFrame(100 * frame + 5, frame)).toBe(100 * frame);
    expect(lastFrame(0, frame)).toBe(0);
    expect(lastFrame(frame, frame)).toBe(0);
  });
});

describe("edit points", () => {
  const tracks = [
    {
      clips: [
        { start: 0, duration: 100 },
        { start: 100, duration: 50 },
      ],
    },
    { clips: [{ start: 30, duration: 40 }] },
    { clips: [] },
  ];
  const points = editPoints(tracks, 150);

  it("collects 0, the end and every clip boundary on all tracks, sorted and unique", () => {
    expect(points).toEqual([0, 30, 70, 100, 150]);
    expect(editPoints([], 0)).toEqual([0]);
  });

  it("finds the previous edit point", () => {
    expect(previousEdit(points, 100)).toBe(70);
    expect(previousEdit(points, 85)).toBe(70);
    expect(previousEdit(points, 30)).toBe(0);
    expect(previousEdit(points, 0)).toBe(0);
  });

  it("finds the next edit point, staying put after the last one", () => {
    expect(nextEdit(points, 0)).toBe(30);
    expect(nextEdit(points, 70)).toBe(100);
    expect(nextEdit(points, 120)).toBe(150);
    expect(nextEdit(points, 150)).toBe(150);
    expect(nextEdit(points, 200)).toBe(200);
  });
});
