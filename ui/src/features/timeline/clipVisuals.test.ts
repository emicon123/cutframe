import { describe, expect, it, vi } from "vitest";

import { FILMSTRIP_FRAME_HEIGHT, type FilmstripLayout } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  buildPeakLevels,
  clipSourceMap,
  drawClipWaveform,
  drawFilmstrip,
  filmstripFrame,
  filmstripSlotWidth,
  peakLevel,
  readPeak,
} from "./clipVisuals";
import type { Viewport } from "./geometry";

const S = FLICKS_PER_SECOND;
/** 1 s between frames, 100 frames, 32 per tile, 16:9 frames (171 × 96). */
const layout: FilmstripLayout = { interval: S, frames: 100, framesPerTile: 32, frameWidth: 171 };
/** 100 px per second, scrolled to 0. */
const viewport: Viewport = { scroll: 0, flicksPerPixel: S / 100 };

/** A forward 1× map from `sourceIn`. */
const forward = (sourceIn: number) => ({ origin: sourceIn, rate: 1 });

/** A stand-in 2D context that records the calls the drawing makes. */
function recorder() {
  return { drawImage: vi.fn(), rect: vi.fn(), beginPath: vi.fn(), fill: vi.fn() };
}
const asCtx = (r: ReturnType<typeof recorder>) => r as unknown as CanvasRenderingContext2D;

describe("filmstrip mapping (F1.34)", () => {
  it("picks frame i for media time i × interval, clamped to the strip", () => {
    expect(filmstripFrame(layout, 0)).toBe(0);
    expect(filmstripFrame(layout, 2.5 * S)).toBe(2);
    expect(filmstripFrame(layout, 500 * S)).toBe(99);
    expect(filmstripFrame(layout, -S)).toBe(0);
  });

  it("keeps the frame aspect in the slot width", () => {
    expect(filmstripSlotWidth(layout, FILMSTRIP_FRAME_HEIGHT)).toBe(171);
    expect(filmstripSlotWidth(layout, 48)).toBe(85.5);
  });

  it("maps the clip's source range onto tiles and frames", () => {
    const ctx = recorder();
    const tiles = { get: vi.fn((_item: string, tile: number) => ({ tile }) as unknown as CanvasImageSource) };
    // Source 30 s in, 2 s long (200 px) at x = 0, 48 px tall: slots 85.5 px wide.
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", forward(30 * S), 0, 200, 3, 48, viewport, 1000);
    // Slots in the middle of 0..85.5, 85.5..171, 171..200 (partial): 30.43 s, 31.28 s, 31.99 s.
    expect(tiles.get.mock.calls.map(([, tile]) => tile)).toEqual([0, 0, 0]);
    const frames = ctx.drawImage.mock.calls.map((call) => (call[1] as number) / 171);
    expect(frames).toEqual([30, 31, 31]);
    expect(ctx.drawImage.mock.calls[0]?.slice(5)).toEqual([0, 3, 85.5, 48]);
    expect(ctx.drawImage.mock.calls[2]?.[5]).toBe(171);
  });

  it("crosses into the next tile", () => {
    const ctx = recorder();
    const tiles = { get: vi.fn((_item: string, tile: number) => ({ tile }) as unknown as CanvasImageSource) };
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", forward(31 * S), 0, 200, 0, 48, viewport, 1000);
    expect(tiles.get.mock.calls.map(([, tile]) => tile)).toEqual([0, 1, 1]);
    // Frame 32 is column 0 of tile 1.
    expect(ctx.drawImage.mock.calls[1]?.[1]).toBe(0);
  });

  it("draws only the slots inside the lanes and skips tiles not loaded", () => {
    const ctx = recorder();
    const tiles = { get: vi.fn(() => null) };
    // A 100 s clip starting 5000 px left of the view, 1000 px of lanes: about 12 slots.
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", forward(0), -5000, 10_000, 0, 48, viewport, 1000);
    expect(tiles.get.mock.calls.length).toBeLessThanOrEqual(13);
    expect(ctx.drawImage).not.toHaveBeenCalled();
  });
});

describe("source range mapping (F1.79, F1.81, F1.82)", () => {
  const media = (sourceIn: number, speed: number, reverse: boolean) =>
    ({ kind: "media", media: "m", sourceIn, speed, reverse }) as const;

  it("spans sourceIn .. sourceEnd over the clip's width, so 2× shows twice the source", () => {
    // 2 s on the timeline playing 10 s .. 14 s of the media.
    const map = clipSourceMap(
      { duration: 2 * S, source: media(10 * S, 200, false), sourceEnd: 14 * S },
      {
        origin: 0,
        rate: 0,
      },
    );
    expect(map).toEqual({ origin: 10 * S, rate: 2 });
  });

  it("mirrors a reversed clip: its start shows the end of the range", () => {
    const map = clipSourceMap(
      { duration: 4 * S, source: media(10 * S, 50, true), sourceEnd: 12 * S },
      {
        origin: 0,
        rate: 0,
      },
    );
    expect(map).toEqual({ origin: 12 * S, rate: -0.5 });
  });

  it("falls back to 1× without sourceEnd, holds one frame for a freeze, none for colour", () => {
    const out = { origin: 0, rate: 0 };
    expect(clipSourceMap({ duration: S, source: media(3 * S, 100, false), sourceEnd: null }, out)).toEqual({
      origin: 3 * S,
      rate: 1,
    });
    expect(
      clipSourceMap({ duration: S, source: { kind: "freeze", media: "m", sourceTime: 7 * S } }, out),
    ).toEqual({
      origin: 7 * S,
      rate: 0,
    });
    expect(clipSourceMap({ duration: S, source: { kind: "color", color: "#000000ff" } }, out)).toBeNull();
  });

  it("draws a 2× filmstrip over twice the source and a reversed one backwards", () => {
    const tiles = { get: vi.fn((_item: string, tile: number) => ({ tile }) as unknown as CanvasImageSource) };
    const fast = recorder();
    // 2 s wide (200 px), source 30 s .. 34 s: slots at 42.75, 128.25, 199 px → 30.86, 32.57, 33.98 s,
    // frames 30, 32 and 33 (columns 30 of tile 0, 0 and 1 of tile 1).
    drawFilmstrip(
      asCtx(fast),
      layout,
      tiles,
      "m",
      { origin: 30 * S, rate: 2 },
      0,
      200,
      0,
      48,
      viewport,
      1000,
    );
    expect(fast.drawImage.mock.calls.map((call) => (call[1] as number) / 171)).toEqual([30, 0, 1]);
    const back = recorder();
    // Reversed 1× from 34 s down: 33.57, 32.72, 32.01 s, frames 33, 32, 32.
    drawFilmstrip(
      asCtx(back),
      layout,
      tiles,
      "m",
      { origin: 34 * S, rate: -1 },
      0,
      200,
      0,
      48,
      viewport,
      1000,
    );
    expect(back.drawImage.mock.calls.map((call) => (call[1] as number) / 171)).toEqual([1, 0, 0]);
  });

  it("repeats the frame at sourceTime for a freeze frame", () => {
    const tiles = { get: vi.fn((_item: string, tile: number) => ({ tile }) as unknown as CanvasImageSource) };
    const ctx = recorder();
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", { origin: 40 * S, rate: 0 }, 0, 200, 0, 48, viewport, 1000);
    // Frame 40 is column 8 of tile 1, in every slot.
    expect(tiles.get.mock.calls.map(([, tile]) => tile)).toEqual([1, 1, 1]);
    expect(ctx.drawImage.mock.calls.map((call) => (call[1] as number) / 171)).toEqual([8, 8, 8]);
  });
});

describe("waveform levels (F1.35)", () => {
  const peaks = Int8Array.from([-10, 10, -50, 20, -5, 90, -1, 1, -127, 0]);

  it("halves the bucket count per level with the min and max of each pair", () => {
    const { levels } = buildPeakLevels(peaks, 100);
    expect(levels.map((level) => level.length / 2)).toEqual([5, 3, 2, 1]);
    expect([...(levels[1] ?? [])]).toEqual([-50, 20, -5, 90, -127, 0]);
    expect([...(levels[3] ?? [])]).toEqual([-127, 90]);
  });

  it("reads about one bucket per pixel at any zoom", () => {
    expect(peakLevel(0.5, 10)).toBe(0);
    expect(peakLevel(2, 10)).toBe(0);
    expect(peakLevel(4, 10)).toBe(2);
    expect(peakLevel(1000, 10)).toBe(9);
    expect(peakLevel(1000, 1)).toBe(0);
  });

  it("reads the min and max of a range at a level, silence outside", () => {
    const levels = buildPeakLevels(peaks, 100);
    expect({ ...readPeak(levels, 0, 1, 3) }).toEqual({ min: -50, max: 90 });
    expect({ ...readPeak(levels, 1, 0, 4) }).toEqual({ min: -50, max: 90 });
    expect({ ...readPeak(levels, 0, 2, 2) }).toEqual({ min: -5, max: 90 });
    expect({ ...readPeak(levels, 0, 50, 60) }).toEqual({ min: 0, max: 0 });
  });

  it("draws one bar per visible column of the clip's source range", () => {
    // 1 s of full-scale audio at 100 buckets per second.
    const loud = new Int8Array(200);
    for (let b = 0; b < 100; b += 1) {
      loud[2 * b] = -127;
      loud[2 * b + 1] = 127;
    }
    const levels = buildPeakLevels(loud, 100);
    const ctx = recorder();
    // 3 s clip whose source starts at 0.5 s, from x = -50: columns 0..250 of 0..1000.
    drawClipWaveform(asCtx(ctx), levels, forward(S / 2), -50, 300, 10, 40, viewport, 1000);
    expect(ctx.rect).toHaveBeenCalledTimes(250);
    // Column 0 is 0.5 s + 50 px = 1.0 s in: past the audio, silence (a 1 px line).
    expect(ctx.rect.mock.calls[0]).toEqual([0, 30, 1, 1]);
    // Before that the clip is off screen; drawing from x = 0 shows the audio at full height.
    const visible = recorder();
    drawClipWaveform(asCtx(visible), levels, forward(0), 0, 100, 10, 40, viewport, 1000);
    expect(visible.rect.mock.calls[0]).toEqual([0, 10, 1, 40]);
  });

  it("reads the source backwards for a reversed clip and faster at 2×", () => {
    // The first 0.5 s is loud, the rest silent.
    const half = new Int8Array(200);
    for (let b = 0; b < 50; b += 1) {
      half[2 * b] = -127;
      half[2 * b + 1] = 127;
    }
    const levels = buildPeakLevels(half, 100);
    // Reversed over 0 .. 1 s: the loud part is at the clip's end (columns 50..99).
    const back = recorder();
    drawClipWaveform(asCtx(back), levels, { origin: S, rate: -1 }, 0, 100, 10, 40, viewport, 1000);
    const heights = back.rect.mock.calls.map((call) => call[3] as number);
    expect(heights[10]).toBe(1);
    expect(heights[90]).toBe(40);
    // 2× from 0: the loud half fills only the first 25 columns.
    const fast = recorder();
    drawClipWaveform(asCtx(fast), levels, { origin: 0, rate: 2 }, 0, 50, 10, 40, viewport, 1000);
    const fastHeights = fast.rect.mock.calls.map((call) => call[3] as number);
    expect(fastHeights[20]).toBe(40);
    expect(fastHeights[30]).toBe(1);
  });
});
