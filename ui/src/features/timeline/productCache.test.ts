import { describe, expect, it, vi } from "vitest";

import { type LoadResult, ProductCache } from "./productCache";

type Pending = { item: string; index: number; resolve: (result: LoadResult<string>) => void };

function setup(budget = 100, maxInFlight = 2) {
  const pending: Pending[] = [];
  const timers: { callback: () => void; ms: number }[] = [];
  let time = 0;
  const release = vi.fn();
  const onChange = vi.fn();
  const cache = new ProductCache<string>({
    budget,
    maxInFlight,
    load: (item, index) =>
      new Promise((resolve) => pending.push({ item: `${item.id}@${item.previewVersion}`, index, resolve })),
    release,
    onChange,
    now: () => time,
    setTimer: (callback, ms) => timers.push({ callback, ms }),
  });
  const frame = (draw: () => void) => {
    cache.beginFrame();
    draw();
    cache.pump();
  };
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  return { cache, pending, timers, release, onChange, frame, settle, advance: (ms: number) => (time += ms) };
}

const a = { id: "a", previewVersion: "1" };
const b = { id: "b", previewVersion: "1" };

describe("ProductCache", () => {
  it("returns null on a miss, loads outside the frame and serves the value after", async () => {
    const { cache, pending, onChange, frame, settle } = setup();
    let value: string | null = "unset";
    frame(() => (value = cache.get(a, 0)));
    expect(value).toBeNull();
    expect(pending.map((p) => [p.item, p.index])).toEqual([["a@1", 0]]);
    pending[0]?.resolve({ kind: "ready", value: "tile", bytes: 10 });
    await settle();
    expect(onChange).toHaveBeenCalledOnce();
    frame(() => (value = cache.get(a, 0)));
    expect(value).toBe("tile");
    expect(cache.size).toBe(10);
    expect(pending).toHaveLength(1);
  });

  it("starts at most maxInFlight loads, most recently asked first, and forgets what scrolled away", async () => {
    const { cache, pending, frame, settle } = setup(100, 2);
    frame(() => {
      cache.get(a, 0);
      cache.get(a, 1);
      cache.get(a, 2);
    });
    expect(pending.map((p) => p.index)).toEqual([2, 1]);
    // Tile 0 is no longer drawn: it is not loaded when a slot frees up.
    frame(() => cache.get(a, 5));
    pending[0]?.resolve({ kind: "ready", value: "t2", bytes: 1 });
    await settle();
    frame(() => cache.get(a, 5));
    expect(pending.map((p) => p.index)).toEqual([2, 1, 5]);
  });

  it("evicts the least recently drawn products over the budget and releases them", async () => {
    const { cache, pending, release, frame, settle } = setup(25, 4);
    frame(() => {
      cache.get(a, 0);
      cache.get(a, 1);
    });
    for (const p of pending) p.resolve({ kind: "ready", value: `a${p.index}`, bytes: 10 });
    await settle();
    frame(() => cache.get(a, 1)); // a1 drawn more recently than a0.
    frame(() => cache.get(b, 0));
    pending[2]?.resolve({ kind: "ready", value: "b0", bytes: 10 });
    await settle();
    expect(release).toHaveBeenCalledWith("a0");
    expect(cache.size).toBe(20);
    let value: string | null = null;
    frame(() => (value = cache.get(a, 1)));
    expect(value).toBe("a1");
  });

  it("keeps everything drawn in the current frame even over the budget", async () => {
    const { cache, pending, release, frame, settle } = setup(5, 4);
    frame(() => cache.get(a, 0));
    pending[0]?.resolve({ kind: "ready", value: "a0", bytes: 10 });
    await settle();
    expect(release).not.toHaveBeenCalled();
  });

  it("drops a media's products when its previewVersion changes", async () => {
    const { cache, pending, release, frame, settle } = setup();
    frame(() => cache.get(a, 0));
    pending[0]?.resolve({ kind: "ready", value: "old", bytes: 10 });
    await settle();
    let value: string | null = "unset";
    frame(() => (value = cache.get({ id: "a", previewVersion: "2" }, 0)));
    expect(value).toBeNull();
    expect(release).toHaveBeenCalledWith("old");
    expect(cache.size).toBe(0);
    expect(pending[1]?.item).toBe("a@2");
  });

  it("releases a value that arrives after its media changed version", async () => {
    const { cache, pending, release, frame, settle } = setup();
    frame(() => cache.get(a, 0));
    frame(() => cache.get({ id: "a", previewVersion: "2" }, 0));
    pending[0]?.resolve({ kind: "ready", value: "stale", bytes: 10 });
    await settle();
    expect(release).toHaveBeenCalledWith("stale");
    expect(cache.size).toBe(0);
  });

  it("retries a 503 with backoff, then gives up", async () => {
    const { cache, pending, timers, frame, settle, advance } = setup();
    frame(() => cache.get(a, 0));
    pending[0]?.resolve({ kind: "retry" });
    await settle();
    expect(timers.map((t) => t.ms)).toEqual([1000]);
    frame(() => cache.get(a, 0));
    expect(pending).toHaveLength(1); // Not due yet.
    advance(1000);
    frame(() => cache.get(a, 0));
    expect(pending).toHaveLength(2);
    for (let attempt = 1; attempt <= 5; attempt += 1) {
      pending[attempt]?.resolve({ kind: "retry" });
      await settle();
      advance(60_000);
      frame(() => cache.get(a, 0));
    }
    expect(timers.map((t) => t.ms)).toEqual([1000, 2000, 4000, 8000, 16000]);
    expect(pending).toHaveLength(6);
  });

  it("never retries a failed product", async () => {
    const { cache, pending, frame, settle } = setup();
    frame(() => cache.get(a, 0));
    pending[0]?.resolve({ kind: "failed" });
    await settle();
    frame(() => cache.get(a, 0));
    expect(pending).toHaveLength(1);
  });

  it("releases everything on clear", async () => {
    const { cache, pending, release, frame, settle } = setup();
    frame(() => cache.get(a, 0));
    pending[0]?.resolve({ kind: "ready", value: "a0", bytes: 10 });
    await settle();
    cache.clear();
    expect(release).toHaveBeenCalledWith("a0");
    expect(cache.size).toBe(0);
  });
});
