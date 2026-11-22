import { describe, expect, it } from "vitest";

import { reducePeaks } from "./waveformDraw";

const peaks = (pairs: [number, number][]) => Int8Array.from(pairs.flat());

describe("reducePeaks", () => {
  it("takes the min and max of the buckets each column covers", () => {
    const input = peaks([
      [-10, 20],
      [-50, 5],
      [-1, 100],
      [-127, 0],
    ]);
    expect([...reducePeaks(input, 2)]).toEqual([-50, 20, -127, 100]);
    expect([...reducePeaks(input, 1)]).toEqual([-127, 100]);
    expect([...reducePeaks(input, 4)]).toEqual([...input]);
  });

  it("repeats buckets when there are more columns than buckets", () => {
    expect([...reducePeaks(peaks([[-3, 7]]), 3)]).toEqual([-3, 7, -3, 7, -3, 7]);
  });

  it("returns silence for an empty waveform and nothing for no columns", () => {
    expect([...reducePeaks(new Int8Array(0), 2)]).toEqual([0, 0, 0, 0]);
    expect(reducePeaks(peaks([[-1, 1]]), 0).length).toBe(0);
  });
});
