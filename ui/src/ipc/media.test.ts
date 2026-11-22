import { mockConvertFileSrc } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  classifyStatus,
  fetchMediaBytes,
  filmstripTileUrl,
  MAX_MEDIA_RETRIES,
  parseWaveform,
  posterUrl,
  retryDelay,
  waveformUrl,
} from "./media";

function waveformBody(
  pairs: [number, number][],
  header: Partial<{ magic: string; version: number; channels: number; rate: number; count: number }> = {},
) {
  const buffer = new ArrayBuffer(16 + pairs.length * 2);
  const view = new DataView(buffer);
  const magic = header.magic ?? "CFWF";
  for (let i = 0; i < 4; i += 1) view.setUint8(i, magic.charCodeAt(i));
  view.setUint16(4, header.version ?? 1, true);
  view.setUint16(6, header.channels ?? 1, true);
  view.setUint32(8, header.rate ?? 100, true);
  view.setUint32(12, header.count ?? pairs.length, true);
  pairs.forEach(([min, max], i) => {
    view.setInt8(16 + 2 * i, min);
    view.setInt8(17 + 2 * i, max);
  });
  return buffer;
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("media URLs", () => {
  it("builds the poster, filmstrip and waveform routes with the preview version", () => {
    mockConvertFileSrc("windows");
    const item = { id: "0193-ab", previewVersion: "0123456789abcdef" };
    expect(posterUrl(item)).toBe("http://media.localhost/poster/0193-ab?v=0123456789abcdef");
    expect(filmstripTileUrl(item, 3)).toBe(
      "http://media.localhost/filmstrip/0193-ab?tile=3&v=0123456789abcdef",
    );
    expect(waveformUrl(item)).toBe("http://media.localhost/waveform/0193-ab?v=0123456789abcdef");
  });
});

describe("classifyStatus", () => {
  it.each([
    [200, "ready"],
    [503, "retry"],
    [500, "failed"],
    [404, "absent"],
    [400, "absent"],
  ] as const)("maps %i to %s", (status, expected) => {
    expect(classifyStatus(status)).toBe(expected);
  });
});

describe("retryDelay", () => {
  it("backs off exponentially and stops after the retry budget", () => {
    expect([0, 1, 2].map(retryDelay)).toEqual([1000, 2000, 4000]);
    expect(retryDelay(MAX_MEDIA_RETRIES - 1)).not.toBeNull();
    expect(retryDelay(MAX_MEDIA_RETRIES)).toBeNull();
    expect(retryDelay(-1)).toBeNull();
  });
});

describe("parseWaveform", () => {
  it("reads the header and the interleaved min/max pairs", () => {
    const waveform = parseWaveform(
      waveformBody([
        [-127, 127],
        [-3, 4],
      ]),
    );
    expect(waveform?.bucketsPerSecond).toBe(100);
    expect([...(waveform?.peaks ?? [])]).toEqual([-127, 127, -3, 4]);
  });

  it("accepts an empty waveform", () => {
    expect(parseWaveform(waveformBody([]))?.peaks.length).toBe(0);
  });

  it.each([
    ["a short header", new ArrayBuffer(8)],
    ["a wrong magic", waveformBody([[0, 0]], { magic: "RIFF" })],
    ["an unknown version", waveformBody([[0, 0]], { version: 2 })],
    ["several channels", waveformBody([[0, 0]], { channels: 2 })],
    ["zero buckets per second", waveformBody([[0, 0]], { rate: 0 })],
    ["a truncated body", waveformBody([[0, 0]], { count: 5 })],
  ])("rejects %s", (_, buffer) => {
    expect(parseWaveform(buffer)).toBeNull();
  });
});

describe("fetchMediaBytes", () => {
  const response = (status: number, body = new ArrayBuffer(2)) =>
    ({ status, arrayBuffer: () => Promise.resolve(body) }) as unknown as Response;

  it("retries 503 with backoff until the product is ready", async () => {
    vi.useFakeTimers();
    const body = new ArrayBuffer(4);
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(response(503))
      .mockResolvedValueOnce(response(503))
      .mockResolvedValueOnce(response(200, body));
    vi.stubGlobal("fetch", fetch);
    const result = fetchMediaBytes("u");
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetch).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(2000);
    await expect(result).resolves.toBe(body);
    expect(fetch).toHaveBeenCalledTimes(3);
  });

  it("gives up at once on 500 and 404", async () => {
    for (const status of [500, 404]) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(response(status));
      vi.stubGlobal("fetch", fetch);
      await expect(fetchMediaBytes("u")).resolves.toBeNull();
      expect(fetch).toHaveBeenCalledTimes(1);
    }
  });

  it("gives up after the retry budget", async () => {
    vi.useFakeTimers();
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(response(503));
    vi.stubGlobal("fetch", fetch);
    const result = fetchMediaBytes("u");
    await vi.advanceTimersByTimeAsync(60_000);
    await expect(result).resolves.toBeNull();
    expect(fetch).toHaveBeenCalledTimes(MAX_MEDIA_RETRIES + 1);
  });

  it("stops when aborted", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", vi.fn<typeof globalThis.fetch>().mockResolvedValue(response(503)));
    const abort = new AbortController();
    const result = fetchMediaBytes("u", abort.signal);
    const settled = expect(result).rejects.toBeDefined();
    await vi.advanceTimersByTimeAsync(10);
    abort.abort();
    await settled;
  });
});
