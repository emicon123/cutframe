import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type IpcError, mockCommands, type PlaybackTick, type SeekMode } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";
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
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 100, seekGeneration: null });

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

const tick = (overrides: Partial<PlaybackTick> = {}): PlaybackTick => ({
  time: 0,
  state: "paused",
  rate: 0,
  looping: false,
  seek: 1,
  droppedFrames: 0,
  target: { kind: "timeline" },
  ...overrides,
});

describe("reduceTick", () => {
  const base = () => playhead.playheadStore.getState();

  it("mirrors the transport from a tick", () => {
    const next = playhead.reduceTick(
      base(),
      tick({ time: 500, state: "playing", rate: 2, looping: true, seek: 4, droppedFrames: 3 }),
      1000,
    );
    expect(next).toEqual({
      time: 500,
      at: 1000,
      state: "playing",
      rate: 2,
      looping: true,
      droppedFrames: 3,
      seekGeneration: 4,
      source: null,
    });
  });

  it("ignores a tick older than the newest seek generation", () => {
    const s = { ...base(), seekGeneration: 5, time: 900 };
    expect(playhead.reduceTick(s, tick({ seek: 4, time: 100 }), 0)).toBeNull();
    expect(playhead.reduceTick(s, tick({ seek: 5, time: 100 }), 0)?.time).toBe(100);
    expect(playhead.reduceTick(s, tick({ seek: 7 }), 0)?.seekGeneration).toBe(7);
  });

  it("keeps the extrapolation base while the same frame repeats", () => {
    const playing = playhead.reduceTick(base(), tick({ time: 10, state: "playing", rate: 1 }), 100);
    if (!playing) throw new Error("tick ignored");
    expect(playhead.reduceTick(playing, tick({ time: 10, state: "playing", rate: 1 }), 116)?.at).toBe(100);
    expect(playhead.reduceTick(playing, tick({ time: 20, state: "playing", rate: 1 }), 133)?.at).toBe(133);
  });
});

describe("displayTime", () => {
  const second = FLICKS_PER_SECOND;
  const playing = { time: 10 * second, at: 1000, state: "playing" as const, rate: 1 };

  it("extrapolates time + elapsed x rate while playing", () => {
    expect(playhead.displayTime(playing, 1020, 60 * second)).toBe(10 * second + 0.02 * second);
    expect(playhead.displayTime({ ...playing, rate: -4 }, 1010, 60 * second)).toBe(
      10 * second - 0.04 * second,
    );
  });

  it("caps the extrapolation, so a late tick never runs the playhead away", () => {
    const capped = 10 * second + (playhead.MAX_EXTRAPOLATION_MS / 1000) * second;
    expect(playhead.displayTime(playing, 5000, 60 * second)).toBe(capped);
  });

  it("stays at the tick while paused and inside the timeline", () => {
    expect(playhead.displayTime({ ...playing, state: "paused", rate: 0 }, 5000, 60 * second)).toBe(
      10 * second,
    );
    expect(playhead.displayTime(playing, 1040, 10 * second)).toBe(10 * second);
    expect(playhead.displayTime({ ...playing, time: 0, rate: -1 }, 1040, 60 * second)).toBe(0);
  });
});

describe("applyTick", () => {
  it("follows ticks and ignores a stale one after a seek", async () => {
    mockSeek(() => 3);
    playhead.applyTick(tick({ time: 100, seek: 2, state: "playing", rate: 1 }));
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 100, state: "playing", rate: 1 });

    playhead.seekTo(5000, "exact");
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(3));
    playhead.applyTick(tick({ time: 120, seek: 2, state: "playing", rate: 1 }));
    expect(playhead.playheadStore.getState().time).toBe(5000);
    playhead.applyTick(tick({ time: 5010, seek: 3, state: "playing", rate: 1 }));
    expect(playhead.playheadStore.getState().time).toBe(5010);
  });

  it("holds ticks while a seek is unanswered and applies the newest one after it", async () => {
    const release: ((generation: number) => void)[] = [];
    mockSeek(() => new Promise<number>((resolve) => release.push(resolve)));
    playhead.seekTo(700, "exact");
    await vi.waitFor(() => expect(release).toHaveLength(1));

    // The engine's tick for the new seek can arrive before the command's answer.
    playhead.applyTick(tick({ time: 100, seek: 1 }));
    playhead.applyTick(tick({ time: 700, seek: 2 }));
    expect(playhead.playheadStore.getState().time).toBe(700);
    release.shift()?.(2);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(2));
    expect(playhead.playheadStore.getState().time).toBe(700);
  });

  it("holds ticks while a scrub waits for its animation frame", () => {
    mockSeek();
    playhead.seekTo(300, "scrub");
    playhead.applyTick(tick({ time: 0, seek: 9 }));
    expect(playhead.playheadStore.getState().time).toBe(300);
  });
});

describe("stepTo", () => {
  it("shows the expected frame, pauses and remembers the step's generation", async () => {
    const steps: number[] = [];
    mockCommands({
      stepFrames: ({ frames }) => {
        steps.push(frames);
        return 6;
      },
    });
    playhead.applyTick(tick({ time: 100, seek: 5, state: "playing", rate: 1 }));
    playhead.stepTo(-10, 40);
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 40, state: "paused", rate: 0 });
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(6));
    expect(steps).toEqual([-10]);
  });
});

describe("source preview", () => {
  const media = "media-1";

  /** Every `setPlaybackTarget` and transport command in the order Rust receives it. */
  function mockTransport(generations: { target?: number; seek?: number; step?: number } = {}) {
    mockCommands({
      setPlaybackTarget: ({ target }) => {
        log.push(target.kind === "media" ? `target ${target.media}` : "target timeline");
        return generations.target ?? 10;
      },
      seek: ({ time, mode }) => {
        log.push(`${mode} ${time}`);
        return generations.seek ?? 11;
      },
      stepFrames: ({ frames }) => {
        log.push(`step ${frames}`);
        return generations.step ?? 12;
      },
    });
  }

  it("shows the media at 0 at once and keeps the timeline playhead", async () => {
    mockTransport();
    playhead.applyTick(tick({ time: 900, seek: 2 }));
    playhead.previewMedia(media);
    expect(playhead.playheadStore.getState()).toMatchObject({
      time: 900,
      state: "paused",
      source: { media, time: 0, state: "paused" },
    });
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(10));
    expect(log).toEqual([`target ${media}`]);
  });

  it("mirrors media ticks into the source transport, never into the timeline playhead", async () => {
    mockTransport();
    playhead.applyTick(tick({ time: 900, seek: 2 }));
    playhead.previewMedia(media);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(10));

    playhead.applyTick(
      tick({ time: 4000, seek: 10, state: "playing", rate: 1, target: { kind: "media", media } }),
    );
    expect(playhead.playheadStore.getState()).toMatchObject({
      time: 900,
      state: "paused",
      rate: 0,
      source: { media, time: 4000, state: "playing", rate: 1 },
    });
  });

  it("ignores a stale timeline tick from before the target change", async () => {
    mockTransport();
    playhead.applyTick(tick({ time: 900, seek: 2, state: "playing", rate: 1 }));
    playhead.previewMedia(media);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(10));

    playhead.applyTick(tick({ time: 950, seek: 2, state: "playing", rate: 1 }));
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 900, source: { media, time: 0 } });
  });

  it("returns to the timeline, which the next tick confirms at the kept playhead", async () => {
    mockTransport({ target: 13 });
    playhead.applyTick(tick({ time: 900, seek: 2 }));
    playhead.previewMedia(media);
    playhead.previewTimeline();
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 900, source: null });
    await vi.waitFor(() => expect(log).toEqual([`target ${media}`, "target timeline"]));

    playhead.applyTick(tick({ time: 900, seek: 13 }));
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 900, source: null, seekGeneration: 13 });
    // Nothing to do on the timeline already.
    playhead.previewTimeline();
    await Promise.resolve();
    expect(log).toHaveLength(2);
  });

  it("steps and seeks in media time without moving the timeline playhead", async () => {
    mockTransport();
    playhead.applyTick(tick({ time: 900, seek: 2 }));
    playhead.previewMedia(media);
    playhead.stepSource(1);
    playhead.seekSource(300.4);
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 900, source: { time: 300 } });
    await vi.waitFor(() => expect(log).toEqual([`target ${media}`, "step 1", "exact 300"]));
  });

  it("goes back to the timeline before a timeline seek", async () => {
    mockTransport();
    playhead.previewMedia(media);
    playhead.seekTo(5000, "exact");
    expect(playhead.playheadStore.getState()).toMatchObject({ time: 5000, source: null });
    await vi.waitFor(() => expect(log).toEqual([`target ${media}`, "target timeline", "exact 5000"]));
  });

  it("leaves the source preview when the engine refuses it", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockCommands({
      setPlaybackTarget: () => rejectAsTauri({ kind: "invalidEdit", message: "unknown media" }),
    });
    playhead.previewMedia(media);
    expect(playhead.playheadStore.getState().source?.media).toBe(media);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().source).toBeNull());
  });
});
