import { describe, expect, it } from "vitest";

import type { ProjectSettings } from "@/bindings";

import {
  formatAspectRatio,
  formFromSettings,
  FRAME_RATE_PRESETS,
  frameRateOptions,
  parseCanvasSize,
  presetFor,
  selectPreset,
  settingsFromForm,
} from "./settings";

const base: ProjectSettings = {
  width: 1920,
  height: 1080,
  frameRate: { num: 30, den: 1 },
  sampleRate: 48000,
};

describe("parseCanvasSize", () => {
  it.each([
    ["16", 16],
    ["8192", 8192],
    [" 1080 ", 1080],
  ])("accepts %s", (text, value) => {
    expect(parseCanvasSize(text)).toEqual({ ok: true, value });
  });

  it.each([
    ["14", "outOfRange"],
    ["8194", "outOfRange"],
    ["1081", "odd"],
    ["17", "odd"],
    ["", "notInteger"],
    ["12.5", "notInteger"],
    ["-20", "notInteger"],
    ["1e3", "notInteger"],
  ])("rejects %j as %s", (text, error) => {
    expect(parseCanvasSize(text)).toEqual({ ok: false, error });
  });
});

describe("presets", () => {
  it("finds the preset of a size and falls back to custom", () => {
    expect(presetFor(1920, 1080)).toBe("landscapeHd");
    expect(presetFor(3840, 2160)).toBe("landscape4k");
    expect(presetFor(1080, 1920)).toBe("shorts");
    expect(presetFor(1080, 1080)).toBe("square");
    expect(presetFor(1280, 720)).toBe("custom");
  });

  it("fills in the size of a picked preset; Custom keeps the typed size", () => {
    const form = formFromSettings(base);
    expect(selectPreset(form, "shorts")).toMatchObject({ preset: "shorts", width: "1080", height: "1920" });
    expect(selectPreset(form, "custom")).toMatchObject({ preset: "custom", width: "1920", height: "1080" });
  });

  it("offers NTSC rates as exact rationals", () => {
    expect(FRAME_RATE_PRESETS).toContainEqual({ num: 24000, den: 1001 });
    expect(FRAME_RATE_PRESETS).toContainEqual({ num: 30000, den: 1001 });
  });

  it("adds a current rate that is not a preset", () => {
    expect(frameRateOptions({ num: 30, den: 1 })).toBe(FRAME_RATE_PRESETS);
    expect(frameRateOptions({ num: 15, den: 1 }).at(-1)).toEqual({ num: 15, den: 1 });
  });
});

describe("formatAspectRatio", () => {
  it("reduces by the gcd", () => {
    expect(formatAspectRatio(1920, 1080)).toBe("16:9");
    expect(formatAspectRatio(3840, 2160)).toBe("16:9");
    expect(formatAspectRatio(1080, 1920)).toBe("9:16");
    expect(formatAspectRatio(1080, 1080)).toBe("1:1");
    expect(formatAspectRatio(1366, 768)).toBe("683:384");
  });
});

describe("settingsFromForm", () => {
  it("builds settings from a preset and keeps the sample rate", () => {
    const form = { ...selectPreset(formFromSettings(base), "shorts"), frameRate: "30000/1001" };
    expect(settingsFromForm(form, base, FRAME_RATE_PRESETS)).toEqual({
      ok: true,
      settings: { width: 1080, height: 1920, frameRate: { num: 30000, den: 1001 }, sampleRate: 48000 },
    });
  });

  it("reports each invalid dimension", () => {
    const form = { ...formFromSettings(base), preset: "custom" as const, width: "1919", height: "9000" };
    expect(settingsFromForm(form, base, FRAME_RATE_PRESETS)).toEqual({
      ok: false,
      width: "odd",
      height: "outOfRange",
    });
  });

  it("round-trips unchanged settings", () => {
    expect(settingsFromForm(formFromSettings(base), base, FRAME_RATE_PRESETS)).toEqual({
      ok: true,
      settings: base,
    });
  });
});
