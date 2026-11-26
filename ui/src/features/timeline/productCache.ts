/**
 * A bounded cache of decoded `media://` products (filmstrip tiles, waveforms) for the timeline
 * canvas (F1.34, F1.35). The draw path only calls `get`, which never decodes, never awaits and
 * allocates nothing for a cached item; a miss queues a load that runs outside the frame, and the
 * cache calls `onChange` when something new can be drawn.
 *
 * Products are keyed by media id, `previewVersion` and an index (the filmstrip tile; 0 for a
 * waveform). A new `previewVersion` drops the media's old products. Memory is bounded by
 * `budget` bytes: the least recently drawn products are released first.
 */
import { retryDelay } from "@/ipc/media";

/** What a load produced: a decoded value and its size, a temporary miss (`503`), or nothing. */
export type LoadResult<V> =
  { kind: "ready"; value: V; bytes: number } | { kind: "retry" } | { kind: "failed" };

type Status = "queued" | "loading" | "ready" | "waiting" | "failed";

type Slot<V> = {
  status: Status;
  value: V | null;
  bytes: number;
  /** Frame in which `get` last asked for it. */
  used: number;
  /** Retries made after `503`. */
  attempt: number;
  /** While `waiting`: when the next try may start (`now()` time). */
  retryAt: number;
};

type MediaEntry<V> = { version: string; slots: Map<number, Slot<V>> };

export type ProductKey = { id: string; previewVersion: string };

export type ProductCacheOptions<V> = {
  /** Upper bound of `bytes` over all ready products. */
  budget: number;
  /** Loads run at the same time; the most recently asked ones start first. */
  maxInFlight: number;
  load: (item: ProductKey, index: number) => Promise<LoadResult<V>>;
  /** Frees a value that left the cache (e.g. `ImageBitmap.close`). */
  release?: (value: V) => void;
  /** Something changed that a redraw should pick up (a load finished, a retry is due). */
  onChange: () => void;
  now?: () => number;
  setTimer?: (callback: () => void, ms: number) => void;
};

export class ProductCache<V> {
  private readonly media = new Map<string, MediaEntry<V>>();
  private readonly queue: { item: ProductKey; index: number; slot: Slot<V> }[] = [];
  private frame = 0;
  private inFlight = 0;
  private total = 0;
  private readonly now: () => number;
  private readonly setTimer: (callback: () => void, ms: number) => void;

  constructor(private readonly options: ProductCacheOptions<V>) {
    this.now = options.now ?? (() => performance.now());
    this.setTimer = options.setTimer ?? ((callback, ms) => void setTimeout(callback, ms));
  }

  /** Bytes held by ready products. */
  get size(): number {
    return this.total;
  }

  /** Starts a new frame: products asked for from now on count as drawn in it. */
  beginFrame(): void {
    this.frame += 1;
  }

  /** The decoded product, or `null` (and a queued load) when it is not ready. */
  get(item: ProductKey, index: number): V | null {
    let entry = this.media.get(item.id);
    if (!entry || entry.version !== item.previewVersion) {
      if (entry) this.dropEntry(entry);
      entry = { version: item.previewVersion, slots: new Map() };
      this.media.set(item.id, entry);
    }
    let slot = entry.slots.get(index);
    if (!slot) {
      slot = { status: "queued", value: null, bytes: 0, used: this.frame, attempt: 0, retryAt: 0 };
      entry.slots.set(index, slot);
      this.queue.push({ item: { id: item.id, previewVersion: item.previewVersion }, index, slot });
      return null;
    }
    slot.used = this.frame;
    if (slot.status === "ready") return slot.value;
    if (slot.status === "waiting" && this.now() >= slot.retryAt) {
      slot.status = "queued";
      this.queue.push({ item: { id: item.id, previewVersion: item.previewVersion }, index, slot });
    }
    return null;
  }

  /**
   * Ends a frame: starts queued loads that were asked for in it, most recent first, up to
   * `maxInFlight`. Queued products no longer drawn (scrolled away) are forgotten, so a fast
   * scroll does not leave a backlog of loads nobody sees.
   */
  pump(): void {
    for (let i = this.queue.length - 1; i >= 0 && this.inFlight < this.options.maxInFlight; i -= 1) {
      const request = this.queue[i];
      if (!request || request.slot.status !== "queued" || request.slot.used < this.frame) continue;
      this.queue.splice(i, 1);
      this.start(request.item, request.index, request.slot);
    }
    for (let i = this.queue.length - 1; i >= 0; i -= 1) {
      const request = this.queue[i];
      if (request && request.slot.status === "queued" && request.slot.used >= this.frame) continue;
      this.queue.splice(i, 1);
      if (request?.slot.status === "queued") this.forget(request.item, request.index, request.slot);
    }
  }

  /** Releases everything (the timeline unmounts). Loads still running are discarded on arrival. */
  clear(): void {
    for (const entry of this.media.values()) this.dropEntry(entry);
    this.media.clear();
    this.queue.length = 0;
  }

  private start(item: ProductKey, index: number, slot: Slot<V>) {
    slot.status = "loading";
    this.inFlight += 1;
    void this.options
      .load(item, index)
      .catch((): LoadResult<V> => ({ kind: "retry" }))
      .then((result) => {
        this.inFlight -= 1;
        this.finish(item, index, slot, result);
        this.options.onChange();
      });
  }

  private finish(item: ProductKey, index: number, slot: Slot<V>, result: LoadResult<V>) {
    const current = this.media.get(item.id)?.slots.get(index) === slot;
    if (!current) {
      // Dropped while loading (new version, cleared): nobody will draw it.
      if (result.kind === "ready") this.options.release?.(result.value);
      return;
    }
    if (result.kind === "ready") {
      slot.status = "ready";
      slot.value = result.value;
      slot.bytes = result.bytes;
      this.total += result.bytes;
      this.evict();
    } else if (result.kind === "retry") {
      const delay = retryDelay(slot.attempt);
      slot.attempt += 1;
      if (delay === null) {
        slot.status = "failed";
      } else {
        slot.status = "waiting";
        slot.retryAt = this.now() + delay;
        this.setTimer(this.options.onChange, delay);
      }
    } else {
      slot.status = "failed";
    }
  }

  /** Releases the least recently drawn ready products until the total fits the budget. */
  private evict() {
    while (this.total > this.options.budget) {
      let oldest: { entry: MediaEntry<V>; index: number; slot: Slot<V> } | null = null;
      for (const entry of this.media.values()) {
        for (const [index, slot] of entry.slots) {
          if (slot.status === "ready" && (!oldest || slot.used < oldest.slot.used))
            oldest = { entry, index, slot };
        }
      }
      // Everything left was drawn in this frame: keep it rather than thrash.
      if (!oldest || oldest.slot.used >= this.frame) return;
      this.releaseSlot(oldest.slot);
      oldest.entry.slots.delete(oldest.index);
    }
  }

  private forget(item: ProductKey, index: number, slot: Slot<V>) {
    const entry = this.media.get(item.id);
    if (entry?.slots.get(index) === slot) entry.slots.delete(index);
  }

  private dropEntry(entry: MediaEntry<V>) {
    for (const slot of entry.slots.values()) this.releaseSlot(slot);
    entry.slots.clear();
  }

  private releaseSlot(slot: Slot<V>) {
    if (slot.status === "ready" && slot.value !== null) {
      this.total -= slot.bytes;
      this.options.release?.(slot.value);
    }
    slot.value = null;
    slot.status = "failed";
  }
}
