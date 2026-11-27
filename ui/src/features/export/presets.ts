import type { EncoderKind, ExportQuality, ExportResolution, FrameRate, VideoCodec } from "@/bindings";

/**
 * What the dialog's video fields hold (export.md §2, §7). Like `VideoExportSettings`, but the
 * custom bitrate is a separate text field, so `quality` is only the choice.
 */
export type VideoChoice = {
  codec: VideoCodec;
  resolution: ExportResolution;
  /** `null`: the project rate (60 fps if the project is faster). */
  frameRate: FrameRate | null;
  quality: ExportQuality["kind"];
  /** `null`: automatic, the best available encoder with fallback. */
  encoder: EncoderKind | null;
};

export type ExportPresetId = "youtube1080" | "youtube4k" | "shorts";

/** What the preset field shows: a preset, or Custom once a field no longer matches one. */
export type PresetLabel = ExportPresetId | "custom";

export const PRESET_IDS: readonly ExportPresetId[] = ["youtube1080", "youtube4k", "shorts"];

const youtube = (resolution: ExportResolution): VideoChoice => ({
  codec: "h264",
  resolution,
  frameRate: null,
  quality: "recommended",
  encoder: null,
});

/**
 * The three presets. Pure data: every derived number (size, bitrate, file size) comes from Rust's
 * `checkExport`. Shorts is a 1080 short side, so a portrait canvas becomes 1080 × 1920.
 */
export const PRESETS: Readonly<Record<ExportPresetId, VideoChoice>> = {
  youtube1080: youtube("p1080"),
  youtube4k: youtube("p2160"),
  shorts: youtube("p1080"),
};

/** Default preset from the canvas: portrait → Shorts, short side ≥ 2160 → 4K, else 1080p. */
export function defaultPreset(canvas: { width: number; height: number }): ExportPresetId {
  if (canvas.height > canvas.width) return "shorts";
  return Math.min(canvas.width, canvas.height) >= 2160 ? "youtube4k" : "youtube1080";
}

export function sameFrameRate(a: FrameRate | null, b: FrameRate | null): boolean {
  if (a === null || b === null) return a === b;
  return a.num * b.den === b.num * a.den;
}

function sameChoice(a: VideoChoice, b: VideoChoice): boolean {
  return (
    a.codec === b.codec &&
    a.resolution === b.resolution &&
    sameFrameRate(a.frameRate, b.frameRate) &&
    a.quality === b.quality &&
    a.encoder === b.encoder
  );
}

/**
 * The label for `choice`. YouTube 1080p and Shorts share their settings, so the picked preset
 * (`picked`) wins when it still matches; any custom change shows Custom.
 */
export function presetLabel(choice: VideoChoice, picked: ExportPresetId): PresetLabel {
  if (sameChoice(choice, PRESETS[picked])) return picked;
  return PRESET_IDS.find((id) => sameChoice(choice, PRESETS[id])) ?? "custom";
}
