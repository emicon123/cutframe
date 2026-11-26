import { describe, expect, it, vi } from "vitest";

import { FILMSTRIP_FRAME_HEIGHT, type FilmstripLayout } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  buildPeakLevels,
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
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", 30 * S, 0, 200, 3, 48, viewport, 1000);
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
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", 31 * S, 0, 200, 0, 48, viewport, 1000);
    expect(tiles.get.mock.calls.map(([, tile]) => tile)).toEqual([0, 1, 1]);
    // Frame 32 is column 0 of tile 1.
    expect(ctx.drawImage.mock.calls[1]?.[1]).toBe(0);
  });

  it("draws only the slots inside the lanes and skips tiles not loaded", () => {
    const ctx = recorder();
    const tiles = { get: vi.fn(() => null) };
    // A 100 s clip starting 5000 px left of the view, 1000 px of lanes: about 12 slots.
    drawFilmstrip(asCtx(ctx), layout, tiles, "m", 0, -5000, 10_000, 0, 48, viewport, 1000);
    expect(tiles.get.mock.calls.length).toBeLessThanOrEqual(13);
    expect(ctx.drawImage).not.toHaveBeenCalled();
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
    drawClipWaveform(asCtx(ctx), levels, S / 2, -50, 300, 10, 40, viewport, 1000);
    expect(ctx.rect).toHaveBeenCalledTimes(250);
    // Column 0 is 0.5 s + 50 px = 1.0 s in: past the audio, silence (a 1 px line).
    expect(ctx.rect.mock.calls[0]).toEqual([0, 30, 1, 1]);
    // Before that the clip is off screen; drawing from x = 0 shows the audio at full height.
    const visible = recorder();
    drawClipWaveform(asCtx(visible), levels, 0, 0, 100, 10, 40, viewport, 1000);
    expect(visible.rect.mock.calls[0]).toEqual([0, 10, 1, 40]);
  });
});
