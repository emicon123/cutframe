import type {
  EncoderKind,
  ExportCapabilities,
  ExportResolution,
  ExportSettings_Deserialize,
  FrameRate,
  ProjectSettings,
  VideoCodec,
} from "@/bindings";
import { t } from "@/i18n";
import { safeFileStem } from "@/lib/fileName";

import { defaultPreset, type ExportPresetId, PRESETS, sameFrameRate, type VideoChoice } from "./presets";

/** Everything the export dialog edits. Text fields keep the raw text the user typed. */
export type ExportForm = {
  /** File name without the folder; `.mp4` is added when it does not end in it. */
  name: string;
  folder: string;
  /** The preset picked last; the shown label is derived (`presetLabel`). */
  preset: ExportPresetId;
  video: VideoChoice;
  /** Custom bitrate in Mbps, used when `video.quality` is `custom`. */
  customMbps: string;
};

export const RESOLUTIONS: readonly ExportResolution[] = [
  "project",
  "p480",
  "p720",
  "p1080",
  "p1440",
  "p2160",
];

export const QUALITIES: readonly VideoChoice["quality"][] = ["low", "recommended", "high", "custom"];

/** The fixed export rates (export.md §4.1); the project rate is offered as well when ≤ 60 fps. */
const FIXED_RATES: readonly FrameRate[] = [24, 25, 30, 50, 60].map((fps) => ({ num: fps, den: 1 }));

const MAX_PROJECT_RATE: FrameRate = { num: 60, den: 1 };

/** The custom bitrate field starts at the 1080p30 recommendation. */
const DEFAULT_CUSTOM_MBPS = "12";

export function initialForm(projectName: string, settings: ProjectSettings, folder: string): ExportForm {
  const preset = defaultPreset(settings);
  return {
    name: safeFileStem(projectName, t.file.untitled),
    folder,
    preset,
    video: PRESETS[preset],
    customMbps: DEFAULT_CUSTOM_MBPS,
  };
}

/** The form after picking a preset: its video settings; name, folder and custom text stay. */
export function applyPreset(form: ExportForm, preset: ExportPresetId): ExportForm {
  return { ...form, preset, video: PRESETS[preset] };
}

/** What "Project" means for the frame rate: the project rate, or 60 fps above that (§4.1). */
export function projectExportRate(projectRate: FrameRate): FrameRate {
  return projectRate.num > 60 * projectRate.den ? MAX_PROJECT_RATE : projectRate;
}

/** The explicit rates to offer next to "Project"; the one "Project" already means is left out. */
export function fixedRateOptions(projectRate: FrameRate): readonly FrameRate[] {
  const project = projectExportRate(projectRate);
  return FIXED_RATES.filter((rate) => !sameFrameRate(rate, project));
}

/** Stable `<select>` value of a frame-rate choice. */
export function frameRateValue(rate: FrameRate | null): string {
  return rate === null ? "project" : `${rate.num}/${rate.den}`;
}

export function parseFrameRateValue(value: string): FrameRate | null {
  const match = /^(\d+)\/(\d+)$/.exec(value);
  return match ? { num: Number(match[1]), den: Number(match[2]) } : null;
}

/** The encoders of `codec` that this machine's probe found usable, in priority order. */
export function availableEncoders(caps: ExportCapabilities | null, codec: VideoCodec): EncoderKind[] {
  return (caps?.encoders ?? []).filter((e) => e.codec === codec && e.available).map((e) => e.encoder);
}

/** HEVC is offered only with a hardware HEVC encoder, so only after the probe found one. */
export function hevcAvailable(caps: ExportCapabilities | null): boolean {
  return availableEncoders(caps, "hevc").length > 0;
}

/** The choice after switching the codec: an encoder that cannot do the new codec goes back to automatic. */
export function withCodec(
  choice: VideoChoice,
  codec: VideoCodec,
  caps: ExportCapabilities | null,
): VideoChoice {
  const encoder =
    choice.encoder !== null && availableEncoders(caps, codec).includes(choice.encoder)
      ? choice.encoder
      : null;
  return { ...choice, codec, encoder };
}

/** A custom bitrate typed in Mbps (`,` or `.` as the decimal mark) → whole kbps, or `null`. */
export function parseMbps(text: string): number | null {
  const normalized = text.trim().replace(",", ".");
  if (!/^\d+(\.\d+)?$/.test(normalized)) return null;
  const kbps = Math.round(Number(normalized) * 1000);
  return kbps > 0 ? kbps : null;
}

/** The output file name: `.mp4` is added unless the name already ends in it (any case). */
export function exportFileName(name: string): string {
  const trimmed = name.trim();
  return /\.mp4$/i.test(trimmed) ? trimmed : `${trimmed}.mp4`;
}

/** Folder + file name as one Windows path. Separators at the end of the folder are not doubled. */
export function joinExportPath(folder: string, name: string): string {
  const dir = folder.trim().replace(/[\\/]+$/, "");
  return `${dir}\\${exportFileName(name)}`;
}

/** A field that must be fixed before Rust is asked. */
export type FormError = "name" | "folder" | "bitrate";

export type ExportRequest =
  { ok: true; settings: ExportSettings_Deserialize } | { ok: false; error: FormError };

/** The `checkExport` / `startExport` settings for the form, or the first field to fix. */
export function buildRequest(form: ExportForm): ExportRequest {
  if (form.name.trim() === "") return { ok: false, error: "name" };
  if (form.folder.trim() === "") return { ok: false, error: "folder" };
  const { codec, resolution, frameRate, quality, encoder } = form.video;
  let kbps = 0;
  if (quality === "custom") {
    const parsed = parseMbps(form.customMbps);
    if (parsed === null) return { ok: false, error: "bitrate" };
    kbps = parsed;
  }
  return {
    ok: true,
    settings: {
      path: joinExportPath(form.folder, form.name),
      target: {
        kind: "video",
        settings: {
          codec,
          resolution,
          frameRate,
          quality: quality === "custom" ? { kind: "custom", kbps } : { kind: quality },
          encoder,
        },
      },
    },
  };
}
