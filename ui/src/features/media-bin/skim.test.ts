import { describe, expect, it } from "vitest";

import type { FilmstripLayout } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import { skimFrame, tileBackground } from "./skim";

// 70 frames: tiles 0 and 1 hold 32 each, tile 2 holds the last 6.
const layout: FilmstripLayout = {
  interval: FLICKS_PER_SECOND,
  frames: 70,
  framesPerTile: 32,
  frameWidth: 54,
};

describe("skimFrame", () => {
  it("maps the left edge to frame 0 and the right edge to the last frame", () => {
    expect(skimFrame(layout, 0)).toEqual({ frame: 0, tile: 0, slot: 0, framesInTile: 32 });
    expect(skimFrame(layout, 1)).toEqual({ frame: 69, tile: 2, slot: 5, framesInTile: 6 });
  });

  it("splits frames into tiles of framesPerTile", () => {
    expect(skimFrame(layout, 31.5 / 70)).toMatchObject({ frame: 31, tile: 0, slot: 31 });
    expect(skimFrame(layout, 32 / 70)).toMatchObject({ frame: 32, tile: 1, slot: 0, framesInTile: 32 });
    expect(skimFrame(layout, 0.5)).toMatchObject({ frame: 35, tile: 1, slot: 3 });
  });

  it("clamps positions outside the thumbnail and ignores NaN", () => {
    expect(skimFrame(layout, -0.3)?.frame).toBe(0);
    expect(skimFrame(layout, 1.7)?.frame).toBe(69);
    expect(skimFrame(layout, Number.NaN)?.frame).toBe(0);
  });

  it("handles a one-frame filmstrip and rejects an empty one", () => {
    const one = { ...layout, frames: 1 };
    expect(skimFrame(one, 0.9)).toEqual({ frame: 0, tile: 0, slot: 0, framesInTile: 1 });
    expect(skimFrame({ ...layout, frames: 0 }, 0.5)).toBeNull();
  });
});

describe("tileBackground", () => {
  it("scales the strip to whole frames and offsets to the slot", () => {
    expect(tileBackground({ slot: 0, framesInTile: 32 })).toEqual({
      backgroundSize: "3200% 100%",
      backgroundPosition: "0% 0%",
    });
    expect(tileBackground({ slot: 31, framesInTile: 32 }).backgroundPosition).toBe("100% 0%");
    expect(tileBackground({ slot: 2, framesInTile: 5 }).backgroundPosition).toBe("50% 0%");
    expect(tileBackground({ slot: 0, framesInTile: 1 })).toEqual({
      backgroundSize: "100% 100%",
      backgroundPosition: "0% 0%",
    });
  });
});
