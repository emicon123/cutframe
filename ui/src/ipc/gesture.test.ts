import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type EditPhase, type IpcError, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useToastStore } from "@/stores/toastStore";

import { beginGesture, type Gesture, startPointerGesture } from "./gesture";

// A manual animation-frame clock: callbacks run only when a test calls `nextFrame()`.
let frames: Map<number, FrameRequestCallback>;
let nextFrameId: number;

function nextFrame() {
  const due = [...frames.values()];
  frames.clear();
  for (const callback of due) callback(0);
}

/** Every IPC call in the order Rust would receive it. */
let log: string[];
let gesture: Gesture<number> | undefined;

function send(value: number, phase: EditPhase): Promise<null> {
  log.push(`${phase} ${value}`);
  return Promise.resolve(null);
}

/** A sender whose calls stay in flight until the test releases them. */
function heldSender() {
  const release: (() => void)[] = [];
  const sender = (value: number, phase: EditPhase) => {
    log.push(`${phase} ${value}`);
    return new Promise<null>((resolve) => release.push(() => resolve(null)));
  };
  return { sender, releaseNext: () => release.shift()?.() };
}

const invalidEdit: IpcError = { kind: "invalidEdit", message: "clip overlaps" };

/** Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`. */
function rejectAsTauri(error: IpcError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
  return Promise.reject(error);
}

beforeEach(() => {
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
    cancelGesture: () => {
      log.push("cancel");
      return null;
    },
  });
});

afterEach(async () => {
  // Leave no gesture active and no IPC call queued for the next test.
  await gesture?.cancel();
  gesture = undefined;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("gesture updates", () => {
  it("sends at most one update per animation frame, with the latest value", async () => {
    gesture = beginGesture(send);
    gesture.update(1);
    gesture.update(2);
    gesture.update(3);
    expect(log).toEqual([]);

    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 3"]));

    nextFrame();
    gesture.update(4);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 3", "update 4"]));
  });

  it("keeps one update in flight and sends the newest value after it settles", async () => {
    const { sender, releaseNext } = heldSender();
    gesture = beginGesture(sender);
    gesture.update(1);
    nextFrame();
    gesture.update(2);
    gesture.update(3);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 1"]));
    expect(frames.size).toBe(0);

    releaseNext();
    await vi.waitFor(() => expect(frames.size).toBe(1));
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 1", "update 3"]));
    releaseNext();
  });

  it("keeps going after a rejected update", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    gesture = beginGesture((value: number, phase) => {
      log.push(`${phase} ${value}`);
      return value === 1 ? rejectAsTauri(invalidEdit) : Promise.resolve(null);
    });
    gesture.update(1);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 1"]));
    gesture.update(2);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 1", "update 2"]));
    expect(gesture.active).toBe(true);
  });
});

describe("gesture commit", () => {
  it("drops the pending update and commits after the update in flight", async () => {
    const { sender, releaseNext } = heldSender();
    gesture = beginGesture(sender);
    gesture.update(1);
    nextFrame();
    gesture.update(2);
    const committed = gesture.commit(5);
    expect(gesture.active).toBe(false);
    await vi.waitFor(() => expect(log).toEqual(["update 1"]));
    await Promise.resolve();
    expect(log).toEqual(["update 1"]);

    releaseNext();
    await vi.waitFor(() => expect(log).toEqual(["update 1", "commit 5"]));
    releaseNext();
    await expect(committed).resolves.toBe(true);

    nextFrame();
    gesture.update(6);
    nextFrame();
    expect(log).toEqual(["update 1", "commit 5"]);
  });

  it("is a single commit without updates (keyboard nudge, typed value)", async () => {
    await expect(beginGesture(send).commit(7)).resolves.toBe(true);
    expect(log).toEqual(["commit 7"]);
  });

  it("cancels the Rust gesture and reports the error when the commit fails", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    gesture = beginGesture((value: number, phase) => {
      log.push(`${phase} ${value}`);
      return phase === "commit" ? rejectAsTauri(invalidEdit) : Promise.resolve(null);
    });
    gesture.update(1);
    nextFrame();

    await expect(gesture.commit(2)).resolves.toBe(false);
    expect(log).toEqual(["update 1", "commit 2", "cancel"]);
    expect(useToastStore.getState().toasts.at(-1)?.text).toBe(t.errors.kinds.invalidEdit);
  });

  it("does nothing after the gesture ended", async () => {
    gesture = beginGesture(send);
    await gesture.cancel();
    await expect(gesture.commit(1)).resolves.toBe(false);
    expect(log).toEqual(["cancel"]);
  });
});

describe("gesture cancel", () => {
  it("cancels on Escape after the update in flight, and stops listening", async () => {
    const { sender, releaseNext } = heldSender();
    gesture = beginGesture(sender);
    gesture.update(1);
    nextFrame();
    gesture.update(2);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(gesture.active).toBe(false);
    await vi.waitFor(() => expect(log).toEqual(["update 1"]));

    releaseNext();
    await vi.waitFor(() => expect(log).toEqual(["update 1", "cancel"]));
    nextFrame();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await Promise.resolve();
    expect(log).toEqual(["update 1", "cancel"]);
  });

  it("ignores other keys", () => {
    gesture = beginGesture(send);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    expect(gesture.active).toBe(true);
  });

  it("cancels on window blur", async () => {
    gesture = beginGesture(send);
    window.dispatchEvent(new Event("blur"));
    expect(gesture.active).toBe(false);
    await vi.waitFor(() => expect(log).toEqual(["cancel"]));
  });

  it("cancels the previous gesture when a new one begins", async () => {
    const first = beginGesture(send);
    gesture = beginGesture(send);
    expect(first.active).toBe(false);
    gesture.update(3);
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["cancel", "update 3"]));
    expect(gesture.active).toBe(true);
  });
});

describe("startPointerGesture", () => {
  function pointerTarget() {
    const element = document.createElement("div");
    const capture = vi.fn();
    element.setPointerCapture = capture;
    return { element, capture };
  }

  const pointer = (type: string, clientX: number, pointerId = 1) =>
    new PointerEvent(type, { pointerId, clientX });

  const startDrag = (element: HTMLElement) =>
    startPointerGesture(element, 1, { send, valueAt: (event) => 100 + event.clientX });

  it("captures the pointer, updates on move and commits on pointer-up", async () => {
    const { element, capture } = pointerTarget();
    gesture = startDrag(element);
    expect(capture).toHaveBeenCalledWith(1);

    element.dispatchEvent(pointer("pointermove", 5));
    element.dispatchEvent(pointer("pointermove", 9, 2)); // another pointer
    nextFrame();
    await vi.waitFor(() => expect(log).toEqual(["update 105"]));

    element.dispatchEvent(pointer("pointerup", 12));
    element.dispatchEvent(pointer("lostpointercapture", 12)); // follows every pointer-up
    await vi.waitFor(() => expect(log).toEqual(["update 105", "commit 112"]));
    await Promise.resolve();
    expect(log).toEqual(["update 105", "commit 112"]);
  });

  it("cancels on lost pointer capture", async () => {
    const { element } = pointerTarget();
    gesture = startDrag(element);
    element.dispatchEvent(pointer("pointermove", 5));
    element.dispatchEvent(pointer("lostpointercapture", 5));
    expect(gesture.active).toBe(false);
    await vi.waitFor(() => expect(log).toEqual(["cancel"]));

    element.dispatchEvent(pointer("pointerup", 8));
    await Promise.resolve();
    expect(log).toEqual(["cancel"]);
  });

  it("cancels on pointercancel", async () => {
    const { element } = pointerTarget();
    gesture = startDrag(element);
    element.dispatchEvent(pointer("pointercancel", 0));
    await vi.waitFor(() => expect(log).toEqual(["cancel"]));
  });
});
