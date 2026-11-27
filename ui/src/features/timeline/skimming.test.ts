import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type Time } from "@/bindings";
import { playheadStore } from "@/stores/playheadStore";
import { useUiStore } from "@/stores/uiStore";

import { createSkimmer, createTimelineSkimmer } from "./skimming";

let frames: Map<number, () => void>;
let nextId: number;
let sent: (Time | null)[];
let allowed: boolean;

const nextFrame = () => {
  const due = [...frames.values()];
  frames.clear();
  for (const callback of due) callback();
};

const skimmer = () =>
  createSkimmer({
    send: (time) => sent.push(time),
    canSkim: () => allowed,
    requestFrame: (callback) => {
      const id = nextId++;
      frames.set(id, callback);
      return id;
    },
    cancelFrame: (id) => frames.delete(id),
  });

beforeEach(() => {
  frames = new Map();
  nextId = 1;
  sent = [];
  allowed = true;
});

describe("createSkimmer", () => {
  it("sends at most one skim per animation frame with the latest integer time", () => {
    const s = skimmer();
    s.hover(10.4);
    s.hover(20.6);
    s.hover(-5);
    expect(sent).toEqual([]);
    expect(frames.size).toBe(1);
    nextFrame();
    expect(sent).toEqual([0]);
    s.hover(30);
    nextFrame();
    expect(sent).toEqual([0, 30]);
  });

  it("does not repeat the time on screen", () => {
    const s = skimmer();
    s.hover(30);
    nextFrame();
    s.hover(30);
    nextFrame();
    expect(sent).toEqual([30]);
  });

  it("sends null when the pointer leaves, once, and drops a pending skim", () => {
    const s = skimmer();
    s.hover(30);
    nextFrame();
    s.hover(40);
    s.leave();
    s.leave();
    nextFrame();
    expect(sent).toEqual([30, null]);
  });

  it("sends nothing on leave when nothing was skimmed", () => {
    const s = skimmer();
    s.hover(30);
    s.leave();
    nextFrame();
    expect(sent).toEqual([]);
  });

  it("stops skimming when it is no longer allowed (playing, toggle off)", () => {
    const s = skimmer();
    s.hover(30);
    nextFrame();
    allowed = false;
    s.hover(40);
    expect(sent).toEqual([30, null]);
    expect(frames.size).toBe(0);

    // Not allowed any more by the time the frame comes.
    allowed = true;
    s.hover(50);
    allowed = false;
    nextFrame();
    expect(sent).toEqual([30, null]);
  });

  it("forgets the skim the engine cleared itself, without sending", () => {
    const s = skimmer();
    s.hover(30);
    nextFrame();
    s.reset();
    s.leave();
    expect(sent).toEqual([30]);
  });
});

describe("createTimelineSkimmer", () => {
  beforeEach(() => {
    vi.stubGlobal("requestAnimationFrame", (callback: () => void) => {
      const id = nextId++;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    mockCommands({
      setSkim: ({ time }) => {
        sent.push(time);
        return null;
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    useUiStore.setState({ skimming: true });
    playheadStore.setState({ state: "paused", rate: 0, source: null });
  });

  it("skims only while on, paused and on the timeline", async () => {
    const s = createTimelineSkimmer();
    useUiStore.setState({ skimming: true });
    s.hover(100);
    nextFrame();
    await vi.waitFor(() => expect(sent).toEqual([100]));

    playheadStore.setState({ state: "playing", rate: 1 });
    s.reset();
    s.hover(200);
    nextFrame();
    playheadStore.setState({
      state: "paused",
      rate: 0,
      source: { media: "m", time: 0, at: 0, state: "paused", rate: 0 },
    });
    s.hover(300);
    nextFrame();
    playheadStore.setState({ source: null });
    useUiStore.setState({ skimming: false });
    s.hover(400);
    nextFrame();
    await Promise.resolve();
    expect(sent).toEqual([100]);
  });
});
