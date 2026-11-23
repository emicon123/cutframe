import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands } from "@/bindings";

type DragModule = typeof import("./playheadDrag");
type PlayheadModule = typeof import("@/stores/playheadStore");

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
let drag: DragModule;
let playhead: PlayheadModule;

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
  mockCommands({
    seek: ({ time, mode }) => {
      log.push(`${mode} ${time}`);
      return log.length;
    },
  });
  // Fresh modules per test: the seek queue and the pending scrub are module state.
  vi.resetModules();
  playhead = await import("@/stores/playheadStore");
  drag = await import("./playheadDrag");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function pointerTarget() {
  const element = document.createElement("div");
  const capture = vi.fn();
  element.setPointerCapture = capture;
  return { element, capture };
}

const pointer = (type: string, clientX: number, pointerId = 1) =>
  new PointerEvent(type, { pointerId, clientX });

/** One pixel is 100 flicks in these tests. */
function press(element: HTMLElement, clientX: number) {
  drag.startPlayheadDrag(element, pointer("pointerdown", clientX), (event) => event.clientX * 100);
}

describe("startPlayheadDrag", () => {
  it("sends one exact seek for a click", async () => {
    const { element, capture } = pointerTarget();
    press(element, 4);
    expect(capture).toHaveBeenCalledWith(1);
    expect(playhead.playheadStore.getState().time).toBe(400);

    element.dispatchEvent(pointer("pointerup", 4));
    element.dispatchEvent(pointer("lostpointercapture", 4)); // follows every pointer-up
    await vi.waitFor(() => expect(log).toEqual(["exact 400"]));
    nextFrame();
    await Promise.resolve();
    expect(log).toEqual(["exact 400"]);
  });

  it("scrubs once per animation frame while dragging and ends with an exact seek", async () => {
    const { element } = pointerTarget();
    press(element, 1);
    element.dispatchEvent(pointer("pointermove", 2));
    element.dispatchEvent(pointer("pointermove", 3));
    element.dispatchEvent(pointer("pointermove", 9, 2)); // another pointer
    expect(playhead.playheadStore.getState().time).toBe(300);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["exact 100", "scrub 300"]));

    element.dispatchEvent(pointer("pointermove", 5));
    element.dispatchEvent(pointer("pointermove", 6));
    element.dispatchEvent(pointer("pointerup", 7));
    element.dispatchEvent(pointer("lostpointercapture", 7));
    // The scrub still pending at release is dropped: the exact seek carries the final time.
    await vi.waitFor(() => expect(log).toEqual(["exact 100", "scrub 300", "exact 700"]));
    expect(frames.size).toBe(0);
    await vi.waitFor(() => expect(playhead.playheadStore.getState().seekGeneration).toBe(3));

    // The drag is over: later moves do nothing.
    element.dispatchEvent(pointer("pointermove", 8));
    nextFrame();
    await Promise.resolve();
    expect(log).toHaveLength(3);
  });

  it("ends a drag on lost capture with an exact seek at the last position", async () => {
    const { element } = pointerTarget();
    press(element, 1);
    element.dispatchEvent(pointer("pointermove", 5));
    element.dispatchEvent(pointer("pointercancel", 0));
    await vi.waitFor(() => expect(log).toEqual(["exact 100", "exact 500"]));
  });
});
