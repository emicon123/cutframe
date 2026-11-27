import { describe, expect, it } from "vitest";

import type { ExportCapabilities, ProjectSettings } from "@/bindings";

import { PRESETS } from "./presets";
import {
  applyPreset,
  availableEncoders,
  buildRequest,
  exportFileName,
  fixedRateOptions,
  frameRateValue,
  hevcAvailable,
  initialForm,
  joinExportPath,
  parseFrameRateValue,
  parseMbps,
  projectExportRate,
  withCodec,
} from "./settings";

const landscape: ProjectSettings = {
  width: 1920,
  height: 1080,
  frameRate: { num: 30, den: 1 },
  sampleRate: 48000,
};

const caps: ExportCapabilities = {
  probed: true,
  encoders: [
    { codec: "h264", encoder: "nvenc", hardware: true, available: true },
    { codec: "h264", encoder: "qsv", hardware: true, available: false },
    { codec: "h264", encoder: "openH264", hardware: false, available: true },
    { codec: "hevc", encoder: "nvenc", hardware: true, available: true },
    { codec: "hevc", encoder: "qsv", hardware: true, available: false },
  ],
};

describe("initialForm", () => {
  it("names the file after the project and picks the preset from the canvas", () => {
    const form = initialForm('My "trip": day 1.', landscape, "D:\\Videos");
    expect(form).toMatchObject({ name: "My _trip__ day 1", folder: "D:\\Videos", preset: "youtube1080" });
    expect(form.video).toEqual(PRESETS.youtube1080);
    expect(initialForm("x", { ...landscape, width: 1080, height: 1920 }, "").preset).toBe("shorts");
  });
});

describe("path", () => {
  it("adds .mp4 unless the name ends in it", () => {
    expect(exportFileName("Trip")).toBe("Trip.mp4");
    expect(exportFileName(" Trip.MP4 ")).toBe("Trip.MP4");
    expect(exportFileName("Trip 2026.11.27")).toBe("Trip 2026.11.27.mp4");
  });

  it("joins folder and name without doubling separators", () => {
    expect(joinExportPath("D:\\Videos", "Trip")).toBe("D:\\Videos\\Trip.mp4");
    expect(joinExportPath("D:\\Videos\\", "Trip.mp4")).toBe("D:\\Videos\\Trip.mp4");
    expect(joinExportPath("C:\\", "a")).toBe("C:\\a.mp4");
    expect(joinExportPath("D:/Videos/", "a")).toBe("D:/Videos\\a.mp4");
  });
});

describe("buildRequest", () => {
  const form = initialForm("Trip", landscape, "D:\\Videos");

  it("maps a preset to the settings Rust takes", () => {
    expect(buildRequest(applyPreset(form, "youtube4k"))).toEqual({
      ok: true,
      settings: {
        path: "D:\\Videos\\Trip.mp4",
        target: {
          kind: "video",
          settings: {
            codec: "h264",
            resolution: "p2160",
            frameRate: null,
            quality: { kind: "recommended" },
            encoder: null,
          },
        },
      },
    });
  });

  it("maps the custom fields, with the bitrate in whole kbps", () => {
    const custom = {
      ...form,
      customMbps: "7,5",
      video: {
        codec: "hevc" as const,
        resolution: "p1440" as const,
        frameRate: { num: 60, den: 1 },
        quality: "custom" as const,
        encoder: "nvenc" as const,
      },
    };
    const request = buildRequest(custom);
    expect(request.ok && request.settings.target.settings).toEqual({
      codec: "hevc",
      resolution: "p1440",
      frameRate: { num: 60, den: 1 },
      quality: { kind: "custom", kbps: 7500 },
      encoder: "nvenc",
    });
    expect(buildRequest({ ...custom, video: { ...custom.video, quality: "low" } })).toMatchObject({
      settings: { target: { settings: { quality: { kind: "low" } } } },
    });
  });

  it("names the field to fix before asking Rust", () => {
    expect(buildRequest({ ...form, name: "  " })).toEqual({ ok: false, error: "name" });
    expect(buildRequest({ ...form, folder: "" })).toEqual({ ok: false, error: "folder" });
    const custom = { ...form, video: { ...form.video, quality: "custom" as const } };
    expect(buildRequest({ ...custom, customMbps: "fast" })).toEqual({ ok: false, error: "bitrate" });
    // Another quality ignores the custom text.
    expect(buildRequest({ ...form, customMbps: "fast" }).ok).toBe(true);
  });
});

describe("parseMbps", () => {
  it("takes dot or comma decimals and rejects anything else", () => {
    expect(parseMbps("12")).toBe(12000);
    expect(parseMbps(" 0.5 ")).toBe(500);
    expect(parseMbps("67,5")).toBe(67500);
    expect(parseMbps("0")).toBeNull();
    expect(parseMbps("-3")).toBeNull();
    expect(parseMbps("1e3")).toBeNull();
    expect(parseMbps("")).toBeNull();
  });
});

describe("frame rates", () => {
  it("offers 24/25/30/50/60 without the one Project already means", () => {
    expect(fixedRateOptions({ num: 30, den: 1 }).map(frameRateValue)).toEqual([
      "24/1",
      "25/1",
      "50/1",
      "60/1",
    ]);
    expect(fixedRateOptions({ num: 30000, den: 1001 })).toHaveLength(5);
  });

  it("caps Project at 60 fps", () => {
    expect(projectExportRate({ num: 120, den: 1 })).toEqual({ num: 60, den: 1 });
    expect(projectExportRate({ num: 30000, den: 1001 })).toEqual({ num: 30000, den: 1001 });
    expect(fixedRateOptions({ num: 120, den: 1 }).map(frameRateValue)).toEqual([
      "24/1",
      "25/1",
      "30/1",
      "50/1",
    ]);
  });

  it("round-trips the select values", () => {
    expect(parseFrameRateValue(frameRateValue(null))).toBeNull();
    expect(parseFrameRateValue(frameRateValue({ num: 25, den: 1 }))).toEqual({ num: 25, den: 1 });
  });
});

describe("encoders", () => {
  it("lists the available encoders of a codec in priority order", () => {
    expect(availableEncoders(caps, "h264")).toEqual(["nvenc", "openH264"]);
    expect(availableEncoders(caps, "hevc")).toEqual(["nvenc"]);
    expect(availableEncoders(null, "h264")).toEqual([]);
  });

  it("offers HEVC only when the probe found a hardware HEVC encoder", () => {
    expect(hevcAvailable(caps)).toBe(true);
    expect(hevcAvailable(null)).toBe(false);
    expect(hevcAvailable({ probed: false, encoders: [] })).toBe(false);
    expect(hevcAvailable({ probed: true, encoders: caps.encoders.filter((e) => e.codec === "h264") })).toBe(
      false,
    );
  });

  it("goes back to automatic when the chosen encoder cannot do the new codec", () => {
    const choice = { ...PRESETS.youtube1080, encoder: "openH264" as const };
    expect(withCodec(choice, "hevc", caps)).toMatchObject({ codec: "hevc", encoder: null });
    expect(withCodec({ ...choice, encoder: "nvenc" }, "hevc", caps)).toMatchObject({
      codec: "hevc",
      encoder: "nvenc",
    });
  });
});
