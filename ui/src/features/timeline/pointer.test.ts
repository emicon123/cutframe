import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type IpcError, mockCommands } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { useAppStore } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { selectionStore } from "@/stores/selectionStore";
import { useUiStore } from "@/stores/uiStore";

import { TRACK_HEIGHT, trackTop } from "./geometry";
import { attachPointerInteraction, type PointerOverlay } from "./pointer";

const S = FLICKS_PER_SECOND;
// 1 px = 1/100 s = one frame at 100 fps.
const viewport = { scroll: 0, flicksPerPixel: S / 100 };
const mid = (index: number) => trackTop(index) + TRACK_HEIGHT / 2;

const project = {
  revision: 1,
  project: {
    mainTrack: "main",
    settings: { frameRate: { num: 100, den: 1 } },
    tracks: [
      {
        id: "main",
        kind: "video",
        locked: false,
        clips: [
          { id: "m1", start: 0, duration: 2 * S, link: null },
          { id: "m2", start: 2 * S, duration: 3 * S, link: null },
        ],
      },
      { id: "locked", kind: "audio", locked: true, clips: [{ id: "a1", start: 0, duration: S, link: null }] },
    ],
  },
} as unknown as ProjectState;

let frames: FrameRequestCallback[];
const nextFrame = () => {
  const due = frames;
  frames = [];
  for (const callback of due) callback(0);
};

let log: string[];
let canvas: HTMLCanvasElement;
let overlay: PointerOverlay;
let detach: () => void;

const pointer = (type: string, x: number, y: number, init: PointerEventInit = {}) =>
  canvas.dispatchEvent(new PointerEvent(type, { pointerId: 1, clientX: x, clientY: y, button: 0, ...init }));

const overlap: IpcError = { kind: "overlap", message: "occupied" };

beforeEach(() => {
  log = [];
  frames = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => frames.push(callback));
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
  mockCommands({
    moveClips: ({ clips, anchor, start, track, phase }) => {
      log.push(`${phase} move ${clips.join(",")} ${anchor} ${start / S} ${JSON.stringify(track)}`);
      return null;
    },
    trimClip: ({ clip, edge, time, phase }) => {
      log.push(`${phase} trim ${clip} ${edge} ${time / S}`);
      return null;
    },
    cancelGesture: () => {
      log.push("cancel");
      return null;
    },
    seek: ({ time, mode }) => {
      log.push(`seek ${mode} ${time / S}`);
      return log.length;
    },
  });
  useProjectStore.setState({ state: project });
  useUiStore.setState({ snapping: true });
  playheadStore.setState({ time: 0 });
  selectionStore.setState({ clips: [], gap: null });

  canvas = document.createElement("canvas");
  let captured = false;
  canvas.setPointerCapture = () => (captured = true);
  canvas.releasePointerCapture = () => (captured = false);
  canvas.hasPointerCapture = () => captured;
  overlay = { marquee: null, snapLine: null };
  detach = attachPointerInteraction(canvas, {
    viewport: () => viewport,
    overlay,
    requestDraw: () => undefined,
  });
});

afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  useProjectStore.setState({ state: null });
  useAppStore.setState({ lastError: null });
});

describe("timeline pointer", () => {
  it("selects on click without sending a gesture", async () => {
    pointer("pointerdown", 300, mid(0));
    pointer("pointermove", 302, mid(0)); // within the 3 px threshold
    pointer("pointerup", 302, mid(0));
    nextFrame();
    await Promise.resolve();
    expect(selectionStore.getState().clips).toEqual(["m2"]);
    expect(log).toEqual([]);

    pointer("pointerdown", 100, mid(0), { ctrlKey: true });
    pointer("pointerup", 100, mid(0), { ctrlKey: true });
    expect(selectionStore.getState().clips).toEqual(["m1", "m2"]);
    pointer("pointerdown", 100, mid(0), { ctrlKey: true });
    pointer("pointerup", 100, mid(0), { ctrlKey: true });
    expect(selectionStore.getState().clips).toEqual(["m2"]);
  });

  it("selects the gap under a click between clips, except on a locked track", () => {
    useProjectStore.setState({
      state: {
        ...project,
        project: {
          ...project.project,
          tracks: [
            {
              id: "main",
              kind: "video",
              locked: false,
              clips: [
                { id: "m1", start: 0, duration: 2 * S, link: null },
                { id: "m2", start: 3 * S, duration: S, link: null },
              ],
            },
            {
              id: "locked",
              kind: "audio",
              locked: true,
              clips: [{ id: "a1", start: S, duration: S, link: null }],
            },
          ],
        },
      } as unknown as ProjectState,
    });
    selectionStore.setState({ clips: ["m1"] });
    pointer("pointerdown", 250, mid(0));
    pointer("pointerup", 250, mid(0));
    expect(selectionStore.getState()).toEqual({ clips: [], gap: { track: "main", at: 2.5 * S } });

    pointer("pointerdown", 50, mid(1));
    pointer("pointerup", 50, mid(1));
    expect(selectionStore.getState()).toEqual({ clips: [], gap: null });
    expect(log).toEqual([]);
  });

  it("clears the selection on a click on empty space", () => {
    selectionStore.setState({ clips: ["m1"] });
    pointer("pointerdown", 900, mid(0));
    pointer("pointerup", 900, mid(0));
    expect(selectionStore.getState().clips).toEqual([]);
  });

  it("moves a clip as one gesture: updates per frame, one commit", async () => {
    pointer("pointerdown", 300, mid(0));
    pointer("pointermove", 340, mid(0));
    pointer("pointermove", 350, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log).toHaveLength(1));
    pointer("pointerup", 360, mid(0));
    const track = JSON.stringify({ kind: "existing", track: "main" });
    await vi.waitFor(() =>
      expect(log).toEqual([`update move m2 m2 2.5 ${track}`, `commit move m2 m2 2.6 ${track}`]),
    );
  });

  it("trims from an edge and snaps to the playhead unless Alt is held", async () => {
    playheadStore.setState({ time: 4 * S });
    pointer("pointerdown", 499, mid(0));
    pointer("pointermove", 405, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update trim m2 end 4"]));
    expect(overlay.snapLine).toBe(4 * S);
    pointer("pointerup", 405, mid(0), { altKey: true });
    await vi.waitFor(() => expect(log).toEqual(["update trim m2 end 4", "commit trim m2 end 4.06"]));
    expect(overlay.snapLine).toBeNull();
  });

  it("cancels the gesture on Escape and ignores the rest of the drag", async () => {
    pointer("pointerdown", 300, mid(0));
    pointer("pointermove", 340, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log).toHaveLength(1));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true }));
    pointer("pointermove", 360, mid(0));
    pointer("pointerup", 360, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log.at(-1)).toBe("cancel"));
    expect(log).toHaveLength(2);
  });

  it("cancels on lost pointer capture", async () => {
    pointer("pointerdown", 300, mid(0));
    pointer("pointermove", 340, mid(0));
    pointer("lostpointercapture", 340, mid(0));
    await vi.waitFor(() => expect(log).toEqual(["cancel"]));
  });

  it("commits the last accepted value when the final one is rejected", async () => {
    let reject = false;
    mockCommands({
      moveClips: ({ start, phase }) => {
        log.push(`${phase} ${start / S}`);
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Tauri rejects with the IpcError itself
        return reject ? Promise.reject(overlap) : null;
      },
    });
    pointer("pointerdown", 300, mid(0));
    pointer("pointermove", 340, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 2.4"]));
    reject = true;
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    pointer("pointermove", 380, mid(0));
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 2.4", "update 2.8"]));
    await vi.waitFor(() => expect(canvas.style.cursor).toBe("not-allowed"));
    reject = false;
    pointer("pointerup", 390, mid(0));
    await vi.waitFor(() => expect(log).toEqual(["update 2.4", "update 2.8", "commit 2.4"]));
  });

  it("does not drag clips on a locked track", async () => {
    pointer("pointerdown", 50, mid(1));
    pointer("pointermove", 90, mid(1));
    pointer("pointerup", 90, mid(1));
    nextFrame();
    await Promise.resolve();
    expect(log).toEqual([]);
    expect(selectionStore.getState().clips).toEqual(["a1"]);
  });

  it("selects with a marquee, linked groups included", () => {
    pointer("pointerdown", 900, trackTop(0) + 10);
    pointer("pointermove", 150, mid(0));
    expect(overlay.marquee).not.toBeNull();
    expect(selectionStore.getState().clips).toEqual(["m1", "m2"]);
    pointer("pointerup", 150, mid(0));
    expect(overlay.marquee).toBeNull();
    expect(selectionStore.getState().clips).toEqual(["m1", "m2"]);
  });

  it("seeks on the ruler: exact on press, scrub while dragging, exact on release", async () => {
    const seeks = () => log.filter((entry) => entry.startsWith("seek"));
    pointer("pointerdown", 250, 5);
    expect(playheadStore.getState().time).toBe(2.5 * S);
    await vi.waitFor(() => expect(seeks()).toEqual(["seek exact 2.5"]));
    pointer("pointermove", 260, 5);
    nextFrame();
    await vi.waitFor(() => expect(seeks()).toEqual(["seek exact 2.5", "seek scrub 2.6"]));
    pointer("pointerup", 260, 5);
    expect(playheadStore.getState().time).toBe(2.6 * S);
    await vi.waitFor(() => expect(seeks()).toEqual(["seek exact 2.5", "seek scrub 2.6", "seek exact 2.6"]));
  });

  it("a click on the ruler is a single exact seek", async () => {
    pointer("pointerdown", 100, 5);
    pointer("pointerup", 100, 5);
    await vi.waitFor(() => expect(log).toEqual(["seek exact 1"]));
  });
});
