import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import { RULER_HEIGHT, TRACK_HEIGHT, trackTop } from "./geometry";
import {
  clipsInRect,
  DRAG_THRESHOLD,
  hitTest,
  type HitTrack,
  pastThreshold,
  TRIM_EDGE_PIXELS,
} from "./hitTest";

const S = FLICKS_PER_SECOND;
// 1 px = 1/100 s: a 1 s clip is 100 px wide.
const viewport = { scroll: 0, flicksPerPixel: S / 100 };

const tracks: HitTrack[] = [
  {
    id: "t0",
    kind: "video",
    locked: false,
    clips: [
      { id: "a", start: S, duration: S },
      { id: "b", start: 2 * S, duration: S },
      { id: "tiny", start: 5 * S, duration: S / 10 },
    ],
  },
  { id: "t1", kind: "audio", locked: true, clips: [{ id: "c", start: 0, duration: 3 * S }] },
];
const mid = (index: number) => trackTop(index) + TRACK_HEIGHT / 2;

describe("hitTest", () => {
  it("finds the ruler above the lanes", () => {
    expect(hitTest(150, RULER_HEIGHT - 1, tracks, viewport)).toEqual({ kind: "ruler" });
  });

  it("finds a clip's body and its trim edges", () => {
    expect(hitTest(150, mid(0), tracks, viewport)).toMatchObject({ kind: "clip", track: 0, zone: "body" });
    expect(hitTest(100 + TRIM_EDGE_PIXELS, mid(0), tracks, viewport)).toMatchObject({
      clip: { id: "a" },
      zone: "start",
    });
    expect(hitTest(199 - TRIM_EDGE_PIXELS + 1, mid(0), tracks, viewport)).toMatchObject({
      clip: { id: "a" },
      zone: "end",
    });
  });

  it("prefers the later clip where two clips touch", () => {
    expect(hitTest(200, mid(0), tracks, viewport)).toMatchObject({ clip: { id: "b" }, zone: "start" });
  });

  it("keeps a body to grab on a narrow clip", () => {
    // 10 px wide: each edge zone is a third of it.
    expect(hitTest(505, mid(0), tracks, viewport)).toMatchObject({ clip: { id: "tiny" }, zone: "body" });
    expect(hitTest(501, mid(0), tracks, viewport)).toMatchObject({ clip: { id: "tiny" }, zone: "start" });
  });

  it("finds empty lane space and space below the tracks", () => {
    expect(hitTest(50, mid(0), tracks, viewport)).toEqual({ kind: "lane", track: 0 });
    expect(hitTest(50, trackTop(2) + 10, tracks, viewport)).toEqual({ kind: "lane", track: null });
  });

  it("follows scroll and zoom", () => {
    const zoomed = { scroll: S, flicksPerPixel: S / 200 };
    expect(hitTest(100, mid(0), tracks, zoomed)).toMatchObject({ clip: { id: "a" }, zone: "body" });
  });
});

describe("clipsInRect", () => {
  it("returns the clips the rubber band touches, in any drag direction", () => {
    expect(clipsInRect({ x0: 250, y0: mid(0), x1: 150, y1: mid(0) + 5 }, tracks, viewport)).toEqual([
      "a",
      "b",
    ]);
  });

  it("skips locked tracks and rows the band does not reach", () => {
    expect(clipsInRect({ x0: 0, y0: mid(0), x1: 1000, y1: mid(1) }, tracks, viewport)).toEqual([
      "a",
      "b",
      "tiny",
    ]);
    expect(clipsInRect({ x0: 0, y0: 0, x1: 1000, y1: RULER_HEIGHT - 1 }, tracks, viewport)).toEqual([]);
  });
});

describe("pastThreshold", () => {
  it("needs more than the threshold distance", () => {
    expect(pastThreshold(DRAG_THRESHOLD, 0)).toBe(false);
    expect(pastThreshold(DRAG_THRESHOLD, 1)).toBe(true);
  });
});
