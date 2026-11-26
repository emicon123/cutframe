/**
 * The timeline's decoded filmstrip tiles and waveforms, loaded from `media://` (media design §4)
 * and kept in bounded caches (`ProductCache`). Tiles are decoded with `createImageBitmap`, off the
 * draw path; waveforms are parsed and their zoom levels built once on arrival.
 */
import { classifyStatus, filmstripTileUrl, parseWaveform, waveformUrl } from "@/ipc/media";

import { buildPeakLevels, type PeakLevels } from "./clipVisuals";
import { type LoadResult, ProductCache, type ProductKey } from "./productCache";

/** Decoded tiles held at most: about 60 tiles of 32 frames at the widest frame (171 × 96). */
export const TILE_BUDGET_BYTES = 128 * 1024 * 1024;
/** Waveform zoom levels held at most: about 40 hours of audio. */
export const WAVEFORM_BUDGET_BYTES = 48 * 1024 * 1024;
const MAX_IN_FLIGHT = 4;

/** One fetch, no retry loop: a `503` comes back as `retry` and the cache schedules the next try. */
async function fetchOnce(url: string): Promise<LoadResult<Blob>> {
  const response = await fetch(url);
  const status = classifyStatus(response.status);
  if (status === "ready") return { kind: "ready", value: await response.blob(), bytes: 0 };
  return { kind: status === "retry" ? "retry" : "failed" };
}

async function loadTile(item: ProductKey, tile: number): Promise<LoadResult<ImageBitmap>> {
  const result = await fetchOnce(filmstripTileUrl(item, tile));
  if (result.kind !== "ready") return result;
  try {
    const bitmap = await createImageBitmap(result.value);
    return { kind: "ready", value: bitmap, bytes: bitmap.width * bitmap.height * 4 };
  } catch {
    return { kind: "failed" };
  }
}

async function loadWaveform(item: ProductKey): Promise<LoadResult<PeakLevels>> {
  const result = await fetchOnce(waveformUrl(item));
  if (result.kind !== "ready") return result;
  const waveform = parseWaveform(await result.value.arrayBuffer());
  if (!waveform) return { kind: "failed" };
  const levels = buildPeakLevels(waveform.peaks, waveform.bucketsPerSecond);
  const bytes = levels.levels.reduce((sum, level) => sum + level.byteLength, 0);
  return { kind: "ready", value: levels, bytes };
}

/** The two caches the canvas draws from; `onChange` requests a redraw. */
export function createClipMedia(onChange: () => void) {
  const tiles = new ProductCache<ImageBitmap>({
    budget: TILE_BUDGET_BYTES,
    maxInFlight: MAX_IN_FLIGHT,
    load: loadTile,
    release: (bitmap) => bitmap.close(),
    onChange,
  });
  const waveforms = new ProductCache<PeakLevels>({
    budget: WAVEFORM_BUDGET_BYTES,
    maxInFlight: MAX_IN_FLIGHT,
    load: (item) => loadWaveform(item),
    onChange,
  });
  return {
    tiles,
    waveforms,
    beginFrame() {
      tiles.beginFrame();
      waveforms.beginFrame();
    },
    endFrame() {
      tiles.pump();
      waveforms.pump();
    },
    clear() {
      tiles.clear();
      waveforms.clear();
    },
  };
}

export type ClipMedia = ReturnType<typeof createClipMedia>;
