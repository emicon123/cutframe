import { describe, expect, it } from "vitest";

import { t } from "@/i18n";

import { addSample, estimateRemainingMs, formatEta, type ProgressSample } from "./eta";

/** Progress events every 100 ms from `from` ms at `rate` progress per second, starting at `start`. */
function feed(
  samples: ProgressSample[],
  from: number,
  to: number,
  start: number,
  rate: number,
): ProgressSample[] {
  let out = samples;
  for (let at = from; at <= to; at += 100)
    out = addSample(out, { at, progress: start + ((at - from) / 1000) * rate });
  return out;
}

describe("addSample", () => {
  it("keeps only the last 5 s", () => {
    const samples = feed([], 0, 10_000, 0, 0.01);
    expect(samples[0]?.at).toBe(5000);
    expect(samples.at(-1)?.at).toBe(10_000);
  });
});

describe("estimateRemainingMs", () => {
  it("shows nothing in the first 3 s of encoding or before encoding", () => {
    const samples = feed([], 0, 2900, 0, 0.01);
    expect(estimateRemainingMs(samples, 0, 2900)).toBeNull();
    expect(estimateRemainingMs(samples, null, 10_000)).toBeNull();
  });

  it("is exact at a constant rate and counts down between events", () => {
    // 1 % per second: at 10 s, 10 % done, 90 s left.
    const samples = feed([], 0, 10_000, 0, 0.01);
    expect(estimateRemainingMs(samples, 0, 10_000)).toBeCloseTo(90_000, 0);
    expect(estimateRemainingMs(samples, 0, 10_400)).toBeCloseTo(89_600, 0);
  });

  it("forgets a slow start after the window", () => {
    // 10 s at 0.1 %/s (decoder warm-up), then 2 %/s.
    let samples = feed([], 0, 10_000, 0, 0.001);
    samples = feed(samples, 10_100, 20_000, 0.01, 0.02);
    const last = samples.at(-1)?.progress ?? 0;
    expect(estimateRemainingMs(samples, 0, 20_000)).toBeCloseTo(((1 - last) / 0.02) * 1000, -1);
  });

  it("gives no estimate during a stall", () => {
    const samples = feed([], 0, 10_000, 0, 0.01);
    // No progress event for more than 5 s.
    expect(estimateRemainingMs(samples, 0, 15_500)).toBeNull();
    // Events that do not move.
    const flat = feed(samples, 10_100, 16_000, 0.1, 0);
    expect(estimateRemainingMs(flat, 0, 16_000)).toBeNull();
  });

  it("never goes below zero", () => {
    const samples = feed([], 0, 5000, 0.9, 0.02);
    expect(estimateRemainingMs(samples, 0, 9000)).toBe(0);
  });
});

describe("formatEta", () => {
  it("rounds seconds up to steps of 5", () => {
    expect(formatEta(1000)).toBe(t.export.progress.etaSeconds(5));
    expect(formatEta(31_000)).toBe(t.export.progress.etaSeconds(35));
  });

  it("shows minutes and hours", () => {
    expect(formatEta(56_000)).toBe(t.export.progress.etaMinutes(1));
    expect(formatEta(4 * 60_000 + 20_000)).toBe(t.export.progress.etaMinutes(4));
    expect(formatEta(65 * 60_000)).toBe(t.export.progress.etaHours(1, 5));
    expect(formatEta(120 * 60_000)).toBe("About 2 h left");
  });
});
