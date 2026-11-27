import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type EditPhase, mockCommands } from "@/bindings";

import { createSliderGesture, type SliderGesture } from "./sliderGesture";

let frames: Map<number, FrameRequestCallback>;
let nextId: number;
let log: string[];
let slider: SliderGesture;

function nextFrame() {
  const due = [...frames.values()];
  frames.clear();
  for (const callback of due) callback(0);
}

/** Lets queued gesture calls run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function send(value: number, phase: EditPhase): Promise<null> {
  log.push(`${phase} ${value}`);
  return Promise.resolve(null);
}

beforeEach(() => {
  log = [];
  frames = new Map();
  nextId = 1;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextId++;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  mockCommands({
    cancelGesture: () => {
      log.push("cancel");
      return null;
    },
  });
  slider = createSliderGesture();
});

afterEach(async () => {
  slider.cancel();
  await settle();
  vi.unstubAllGlobals();
});

describe("slider gesture phases (ADR-0006)", () => {
  it("sends updates at most once per frame while dragging and commits the last value on release", async () => {
    slider.pointerDown();
    slider.change(110, send);
    slider.change(120, send);
    nextFrame();
    await settle();
    slider.change(130, send);
    slider.pointerUp();
    // Radix reports the commit too; a drag ignores it, so nothing is sent twice.
    slider.commit(130, send);
    await settle();
    nextFrame();
    expect(log).toEqual(["update 120", "commit 130"]);
  });

  it("cancels on Escape and ignores the rest of that drag", async () => {
    slider.pointerDown();
    slider.change(150, send);
    nextFrame();
    await settle();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    slider.change(170, send);
    slider.pointerUp();
    await settle();
    expect(log).toEqual(["update 150", "cancel"]);
  });

  it("cancels when the pointer capture is lost", async () => {
    slider.pointerDown();
    slider.change(150, send);
    slider.cancel();
    await settle();
    nextFrame();
    expect(log).toEqual(["cancel"]);
  });

  it("sends a keyboard step as one commit", async () => {
    slider.keyDown();
    slider.change(101, send);
    slider.commit(101, send);
    await settle();
    nextFrame();
    expect(log).toEqual(["commit 101"]);
  });

  it("sends nothing for a click that moves nothing", async () => {
    slider.pointerDown();
    slider.pointerUp();
    slider.cancel();
    await settle();
    expect(log).toEqual([]);
  });
});
