import { describe, expect, it } from "vitest";

import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  fadeLimit,
  formatDecibels,
  formatDurationSeconds,
  formatFade,
  formatSpeed,
  formatVolume,
  parseDuration,
  parseFade,
  parseNumber,
  parseSpeed,
  parseVolume,
  SPEED_SLIDER_MAX,
  sliderToSpeed,
  speedFromFactor,
  speedToSlider,
  volumeFromSlider,
} from "./values";

const S = FLICKS_PER_SECOND;

describe("parseNumber", () => {
  it("accepts a decimal comma, spaces and a unit sign", () => {
    expect(parseNumber(" 1,5 ")).toBe(1.5);
    expect(parseNumber("200 %")).toBe(200);
    expect(parseNumber("2×")).toBe(2);
    expect(parseNumber("0.5x")).toBe(0.5);
    expect(parseNumber("3s")).toBe(3);
  });

  it("rejects text that is not a finite number", () => {
    expect(parseNumber("")).toBeNull();
    expect(parseNumber("fast")).toBeNull();
    expect(parseNumber("Infinity")).toBeNull();
  });
});

describe("volume (F1.77)", () => {
  it("shows the gain in percent and clamps typed percent to 0–400 %", () => {
    expect(formatVolume(1)).toBe("100");
    expect(formatVolume(0.506)).toBe("51");
    expect(parseVolume("150")).toBe(1.5);
    expect(parseVolume("900")).toBe(4);
    expect(parseVolume("-5")).toBe(0);
    expect(parseVolume("loud")).toBeNull();
  });

  it("snaps a dragged volume to 100 % near it", () => {
    expect(volumeFromSlider(98)).toBe(1);
    expect(volumeFromSlider(103)).toBe(1);
    expect(volumeFromSlider(105)).toBe(1.05);
    expect(volumeFromSlider(400)).toBe(4);
  });

  it("reads out dB with one decimal, −∞ at silence", () => {
    expect(formatDecibels(1)).toBe(t.inspector.audio.decibels("0.0"));
    expect(formatDecibels(4)).toBe(t.inspector.audio.decibels("+12.0"));
    expect(formatDecibels(0.5)).toBe(t.inspector.audio.decibels("−6.0"));
    expect(formatDecibels(0)).toBe(t.inspector.audio.silent);
  });
});

describe("speed (F1.79)", () => {
  it("converts a factor to a rounded, clamped percent", () => {
    expect(speedFromFactor(2)).toBe(200);
    expect(speedFromFactor(0.333)).toBe(33);
    expect(speedFromFactor(1.006)).toBe(101);
    expect(speedFromFactor(0.01)).toBe(10);
    expect(speedFromFactor(250)).toBe(10_000);
  });

  it("shows two decimals and parses typed factors through the clamp", () => {
    expect(formatSpeed(200)).toBe("2.00");
    expect(formatSpeed(25)).toBe("0.25");
    expect(parseSpeed("1,5")).toBe(150);
    expect(parseSpeed("0")).toBe(10);
    expect(parseSpeed("x")).toBeNull();
  });

  it("maps the slider logarithmically from 0.1× to 100×", () => {
    expect(sliderToSpeed(0)).toBe(10);
    expect(sliderToSpeed(SPEED_SLIDER_MAX)).toBe(10_000);
    // Each third of the slider is one decade: 1× and 10×.
    expect(speedToSlider(100)).toBe(333);
    expect(speedToSlider(1000)).toBe(667);
    expect(sliderToSpeed(667)).toBe(1002);
    expect(speedToSlider(10)).toBe(0);
    expect(speedToSlider(5)).toBe(0);
    for (const speed of [10, 25, 50, 200, 400, 2000, 10_000])
      expect(Math.abs(sliderToSpeed(speedToSlider(speed)) - speed) / speed).toBeLessThan(0.01);
  });

  it("snaps a dragged speed to 1× near it", () => {
    expect(sliderToSpeed(328)).toBe(100);
    expect(sliderToSpeed(341)).toBe(100);
    expect(sliderToSpeed(350)).toBe(112);
  });
});

describe("fades and durations (F1.78, F1.83)", () => {
  it("limits a fade to the room left by the other fade", () => {
    expect(fadeLimit(10 * S, 4 * S)).toBe(6 * S);
    expect(fadeLimit(2 * S, 4 * S)).toBe(0);
  });

  it("shows fades in seconds with one decimal and clamps typed ones", () => {
    expect(formatFade(1.25 * S)).toBe("1.3");
    expect(parseFade("1.5", 10 * S)).toBe(1.5 * S);
    expect(parseFade("20", 6 * S)).toBe(6 * S);
    expect(parseFade("-1", 6 * S)).toBe(0);
    expect(parseFade("soft", 6 * S)).toBeNull();
  });

  it("rounds a typed duration to whole frames, at least one", () => {
    const frame = S / 30;
    expect(parseDuration("2", frame)).toBe(2 * S);
    expect(parseDuration("0.51", frame)).toBe(15 * frame);
    expect(parseDuration("0.001", frame)).toBe(frame);
    expect(parseDuration("0", frame)).toBeNull();
    expect(formatDurationSeconds(2.5 * S)).toBe("2.5");
    expect(formatDurationSeconds(5 * S)).toBe("5");
  });
});
