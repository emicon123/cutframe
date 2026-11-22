import { convertFileSrc } from "@tauri-apps/api/core";

import type { MediaView_Serialize } from "@/bindings";

let base: string | undefined;

/**
 * URL of a `media://` resource. The scheme differs per platform (`http://media.localhost/` on
 * Windows, `media://localhost/` on Linux); `convertFileSrc` knows which one applies.
 */
export function mediaUrl(path: string): string {
  base ??= convertFileSrc("", "media");
  return base + path.replace(/^\//, "");
}

type Versioned = Pick<MediaView_Serialize, "id" | "previewVersion">;

const version = (item: Versioned) => encodeURIComponent(item.previewVersion);

/** Poster JPEG (long side `POSTER_LONG_SIDE`, upright) of a video or image. */
export function posterUrl(item: Versioned): string {
  return mediaUrl(`poster/${item.id}?v=${version(item)}`);
}

/** Filmstrip tile `tile` of a video: `FilmstripLayout.framesPerTile` frames side by side. */
export function filmstripTileUrl(item: Versioned, tile: number): string {
  return mediaUrl(`filmstrip/${item.id}?tile=${tile}&v=${version(item)}`);
}

/** Binary waveform (see `parseWaveform`) of audio and of video with audio. */
export function waveformUrl(item: Versioned): string {
  return mediaUrl(`waveform/${item.id}?v=${version(item)}`);
}

/**
 * What a `media://` status means for the UI (design §4): `ready` 200; `retry` 503 (not ready,
 * queue full or the file unreachable: try again later); `failed` 500 (generation failed for this
 * session: show a placeholder, never retry); `absent` 404 and anything else (stale or no such
 * product).
 */
export type MediaStatusClass = "ready" | "retry" | "failed" | "absent";

export function classifyStatus(status: number): MediaStatusClass {
  if (status >= 200 && status < 300) return "ready";
  if (status === 503) return "retry";
  if (status === 500) return "failed";
  return "absent";
}

/** Retries for a `503`, after which the UI shows the placeholder. */
export const MAX_MEDIA_RETRIES = 5;

/** Delay before retry number `attempt` (0-based): 1 s, 2 s, 4 s, …; `null` once retries are used up. */
export function retryDelay(attempt: number): number | null {
  if (attempt < 0 || attempt >= MAX_MEDIA_RETRIES) return null;
  return 1000 * 2 ** attempt;
}

/** The status of a `media://` URL; network failures count as `retry`. */
export async function probeMedia(url: string, signal?: AbortSignal): Promise<MediaStatusClass> {
  try {
    const response = await fetch(url, signal ? { signal } : undefined);
    return classifyStatus(response.status);
  } catch (error) {
    if (signal?.aborted) throw error;
    return "retry";
  }
}

const sleep = (ms: number, signal: AbortSignal | undefined) =>
  new Promise<void>((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        reject(signal.reason instanceof Error ? signal.reason : new DOMException("Aborted", "AbortError"));
      },
      { once: true },
    );
  });

/**
 * Fetches a `media://` body, retrying `503` with backoff. Resolves `null` when the product failed,
 * does not exist or stayed unavailable. Rejects only when `signal` aborts.
 */
export async function fetchMediaBytes(url: string, signal?: AbortSignal): Promise<ArrayBuffer | null> {
  for (let attempt = 0; ; attempt += 1) {
    let status: MediaStatusClass;
    try {
      const response = await fetch(url, signal ? { signal } : undefined);
      status = classifyStatus(response.status);
      if (status === "ready") return await response.arrayBuffer();
    } catch (error) {
      if (signal?.aborted) throw error;
      status = "retry";
    }
    const delay = status === "retry" ? retryDelay(attempt) : null;
    if (delay === null) return null;
    await sleep(delay, signal);
  }
}

/** A decoded `/waveform` body: one mono (min, max) pair per bucket, each in -127..=127. */
export type Waveform = {
  bucketsPerSecond: number;
  /** Interleaved min, max per bucket; length is `2 × bucket count`. */
  peaks: Int8Array;
};

const WAVEFORM_MAGIC = "CFWF";
const WAVEFORM_HEADER = 16;

/**
 * Parses the waveform format of design §4 (little endian): magic `CFWF`, `u16` version (1), `u16`
 * channels (1), `u32` buckets per second, `u32` bucket count, then `count × (i8 min, i8 max)`.
 * Returns `null` for anything else, including a truncated body.
 */
export function parseWaveform(buffer: ArrayBuffer): Waveform | null {
  if (buffer.byteLength < WAVEFORM_HEADER) return null;
  const view = new DataView(buffer);
  const magic = String.fromCharCode(view.getUint8(0), view.getUint8(1), view.getUint8(2), view.getUint8(3));
  if (magic !== WAVEFORM_MAGIC) return null;
  if (view.getUint16(4, true) !== 1 || view.getUint16(6, true) !== 1) return null;
  const bucketsPerSecond = view.getUint32(8, true);
  const count = view.getUint32(12, true);
  if (bucketsPerSecond === 0 || buffer.byteLength < WAVEFORM_HEADER + count * 2) return null;
  return { bucketsPerSecond, peaks: new Int8Array(buffer, WAVEFORM_HEADER, count * 2) };
}
