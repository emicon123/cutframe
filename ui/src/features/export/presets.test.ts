import { describe, expect, it } from "vitest";

import { defaultPreset, presetLabel, PRESETS } from "./presets";

describe("defaultPreset", () => {
  it("picks Shorts for a portrait canvas", () => {
    expect(defaultPreset({ width: 1080, height: 1920 })).toBe("shorts");
    expect(defaultPreset({ width: 2160, height: 3840 })).toBe("shorts");
  });

  it("picks 4K from a short side of 2160 and 1080p below", () => {
    expect(defaultPreset({ width: 3840, height: 2160 })).toBe("youtube4k");
    expect(defaultPreset({ width: 2560, height: 1440 })).toBe("youtube1080");
    expect(defaultPreset({ width: 1920, height: 1080 })).toBe("youtube1080");
    expect(defaultPreset({ width: 1080, height: 1080 })).toBe("youtube1080");
  });
});

describe("presets", () => {
  it("are H.264 at the project rate, recommended quality and automatic encoder", () => {
    for (const preset of Object.values(PRESETS)) {
      expect(preset).toMatchObject({ codec: "h264", frameRate: null, quality: "recommended", encoder: null });
    }
    expect(PRESETS.youtube1080.resolution).toBe("p1080");
    expect(PRESETS.youtube4k.resolution).toBe("p2160");
    expect(PRESETS.shorts.resolution).toBe("p1080");
  });
});

describe("presetLabel", () => {
  it("keeps the picked preset while it matches, also for the twins 1080p and Shorts", () => {
    expect(presetLabel(PRESETS.shorts, "shorts")).toBe("shorts");
    expect(presetLabel(PRESETS.youtube1080, "youtube1080")).toBe("youtube1080");
  });

  it("switches to Custom when any field changes", () => {
    const base = PRESETS.youtube1080;
    expect(presetLabel({ ...base, quality: "high" }, "youtube1080")).toBe("custom");
    expect(presetLabel({ ...base, frameRate: { num: 60, den: 1 } }, "youtube1080")).toBe("custom");
    expect(presetLabel({ ...base, codec: "hevc" }, "youtube1080")).toBe("custom");
    expect(presetLabel({ ...base, encoder: "nvenc" }, "youtube1080")).toBe("custom");
  });

  it("finds another preset the fields match", () => {
    expect(presetLabel({ ...PRESETS.youtube1080, resolution: "p2160" }, "youtube1080")).toBe("youtube4k");
  });
});
