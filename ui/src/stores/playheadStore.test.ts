import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type IpcError, mockCommands, type SeekMode } from "@/bindings";
import { useToastStore } from "@/stores/toastStore";

type PlayheadModule = typeof import("./playheadStore");

// A manual animation-frame clock: callbacks run only when a test calls `nextFrame()`.
let frames: Map<number, FrameRequestCallback>;
let nextFrameId: number;

function nextFrame() {
  const due = [...frames.values()];
  frames.clear();
  for (const callback of due) callback(0);
}

/** Every `seek` in the order Rust receives it. */
let log: string[];
let playhead: PlayheadModule;

function mockSeek(handler?: (time: number, mode: SeekMode) => number | Promise<number>) {
  mockCommands({
    seek: ({ time, mode }) => {
      log.push(`${mode} ${time}`);
      return handler ? handler(time, mode) : log.length;
    },
  });
}

/** Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`. */
function rejectAsTauri(error: IpcError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
  return Promise.reject(error);
}

beforeEach(async () => {
  log = [];
  frames = new Map();
  nextFrameId = 1;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextFrameId++;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  // A fresh module per test: the seek queue and the pending scrub are module state.
  vi.resetModules();
  playhead = await import("./playheadStore");
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("seekTo", () => {
  it("moves the playhead at once and sends an exact seek with integer flicks", async () => {
    mockSeek();
    playhead.seekTo(1234.6, "exact");
    expect(playhead.playheadStore.getState().time).toBe(1235);
    await vi.waitFor(() => expect(log).toEqual(["exact 1235"]));
  });

  it("coalesces scrubs to one seek per animation frame with the latest time", async () => {
    mockSeek();
    playhead.seekTo(10, "scrub");
    playhead.seekTo(20, "scrub");
    playhead.seekTo(30, "scrub");
    expect(playhead.playheadStore.getState().time).toBe(30);
    expect(log).toEqual([]);

    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["scrub 30"]));
    playhead.seekTo(40, "scrub");
    playhead.seekTo(50, "scrub");
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["scrub 30", "scrub 50"]));
  });

  it("keeps at most one scrub in flight", async () => {
    const release: ((generation: number) => void)[] = [];
    mockSeek(() => new Promise<number>((resolve) => release.push(resolve)));
    playhead.seekTo(10, "scrub");
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["scrub 10"]));

    playhead.seekTo(20, "scrub");
    playhead.seekTo(30, "scrub");
    nextFrame();
    await Promise.resolve();
    expect(log).toEqual(["scrub 10"]);

    release.shift()?.(1);
    await vi.waitFor(() => expect(frames.size).toBe(1));
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["scrub 10", "scrub 30"]));
    release.shift()?.(2);
  });

  it("does not send a scrub when the time did not change", () => {
    mockSeek();
    playhead.setPlayhead(10);
    playhead.seekTo(10, "scrub");
    expect(frames.size).toBe(0);
  });

  it("drops a pending scrub for an exact seek, which never arrives before an older scrub", async () => {
    const release: ((generation: number) => void)[] = [];
    mockSeek(() => new Promise<number>((resolve) => release.push(resolve)));
    playhead.seekTo(10, "scrub");
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["scrub 10"]));
    playhead.seekTo(20, "scrub");
    playhead.seekTo(25, "exact");
    expect(frames.size).toBe(0);

    await Promise.resolve();
    expect(log).toEqual(["scrub 10"]); // queued behind the scrub in flight
    release.shift()?.(1);
    await vi.waitFor(() => expect(log).toEqual(["scrub 10", "exact 25"]));
    release.shift()?.(2);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(2));
  });

  it("remembers the generation of the newest answered seek", async () => {
    mockSeek((time) => time / 10);
    expect(playhead.playheadStore.getState().seekGeneration).toBeNull();
    playhead.seekTo(70, "exact");
    playhead.seekTo(90, "exact");
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(9));
  });

  it("logs unavailable at debug level only, with no error for the user", async () => {
    const debug = vi.spyOn(console, "debug").mockImplementation(() => undefined);
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockSeek(() => rejectAsTauri({ kind: "unavailable", message: "no preview engine" }));

    playhead.seekTo(100, "exact");
    await vi.waitFor(() => expect(debug).toHaveBeenCalledTimes(1));
    expect(error).not.toHaveBeenCalled();
    expect(useToastStore.getState().toasts).toEqual([]);
    expect(playhead.playheadStore.getState()).toEqual({ time: 100, seekGeneration: null });

    // The queue goes on after a failure.
    mockSeek();
    playhead.seekTo(200, "exact");
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(2));
  });

  it("logs other failures as errors without a status-bar message", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockSeek(() => rejectAsTauri({ kind: "invalidEdit", message: "time out of range" }));

    playhead.seekTo(100, "exact");
    await vi.waitFor(() => expect(error).toHaveBeenCalledWith("[ipc] invalidEdit: time out of range"));
    expect(useToastStore.getState().toasts).toEqual([]);
  });
});

describe("reseekOnDurationChange", () => {
  it("sends the playhead on the first state and when the old duration was before it", async () => {
    mockSeek();
    playhead.setPlayhead(500);
    playhead.reseekOnDurationChange(undefined, 0);
    playhead.reseekOnDurationChange(0, 0); // unchanged
    playhead.reseekOnDurationChange(0, undefined); // no project
    playhead.reseekOnDurationChange(0, 800);
    playhead.reseekOnDurationChange(800, 300); // the engine kept 500
    await vi.waitFor(() => expect(log).toEqual(["exact 500", "exact 500"]));
  });
});
