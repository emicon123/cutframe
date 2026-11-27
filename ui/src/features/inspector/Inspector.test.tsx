import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";
import { clearSelection, setSelection } from "@/stores/selectionStore";

import { Inspector } from "./Inspector";
import { audio, clip, projectState, S, track } from "./testProject";

/** Every command the inspector sent, as `[command, args]`. */
let calls: [string, unknown][];

const tracks = (locked = false) => [
  track(
    "main",
    "video",
    [
      clip("video", { audio: audio({ volume: { value: 0.5 }, fadeOut: S }) }),
      clip("image", {
        start: 2 * S,
        audio: null,
        source: { kind: "media", media: "img", sourceIn: 0, speed: 100, reverse: false },
      }),
      clip("fast", {
        start: 4 * S,
        source: { kind: "media", media: "v", sourceIn: 0, speed: 500, reverse: true },
      }),
    ],
    locked,
  ),
];

function show(selected: string[], locked = false) {
  useProjectStore.setState({ state: projectState(tracks(locked)) });
  act(() => setSelection(selected));
  render(<Inspector />);
}

const section = (name: string) => within(screen.getByRole("region", { name }));

/** Types into a field and presses Enter. */
function type(field: HTMLElement, text: string) {
  fireEvent.change(field, { target: { value: text } });
  fireEvent.keyDown(field, { key: "Enter" });
  fireEvent.blur(field);
}

/** Lets the gesture queue send its calls. */
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

beforeEach(() => {
  calls = [];
  // Radix measures the slider thumb; jsdom has no ResizeObserver.
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  mockCommands({
    setClipProperty: (args) => (calls.push(["setClipProperty", args]), null),
    setClipSpeed: (args) => (calls.push(["setClipSpeed", args]), null),
    setClipReverse: (args) => (calls.push(["setClipReverse", args]), null),
    trimClip: (args) => (calls.push(["trimClip", args]), null),
    cancelGesture: () => null,
  });
});

afterEach(() => {
  act(() => clearSelection());
  useProjectStore.setState({ state: null });
  vi.unstubAllGlobals();
});

describe("Inspector shell", () => {
  it("shows the project with no clip selected", () => {
    show([]);
    expect(screen.getByRole("region", { name: t.inspector.title })).toBeTruthy();
    expect(screen.getByLabelText(t.inspector.name)).toBeTruthy();
  });

  it("shows the Audio and Speed sections of a video clip, Duration for an image", () => {
    show(["video"]);
    expect(screen.getByRole("heading", { name: t.inspector.clip(1) })).toBeTruthy();
    expect(screen.getByRole("region", { name: t.inspector.audio.title })).toBeTruthy();
    expect(screen.getByRole("region", { name: t.inspector.speed.title })).toBeTruthy();
    expect(screen.queryByRole("region", { name: t.inspector.still.title })).toBeNull();
  });

  it("says when the selected clips share nothing", () => {
    show(["video", "image"]);
    expect(screen.getByText(t.inspector.noCommon)).toBeTruthy();
  });

  it("shows mixed values as mixed", () => {
    show(["video", "fast"]);
    const audioSection = section(t.inspector.audio.title);
    const volume = audioSection.getByLabelText(t.inspector.audio.volume, { selector: "input" });
    expect((volume as HTMLInputElement).value).toBe("");
    expect(volume.getAttribute("placeholder")).toBe(t.inspector.mixed);
    const reverse = section(t.inspector.speed.title).getByLabelText(t.inspector.speed.reverse);
    expect((reverse as HTMLInputElement).indeterminate).toBe(true);
  });

  it("disables the controls of a clip on a locked track", () => {
    show(["video"], true);
    expect(screen.getByText(t.inspector.locked)).toBeTruthy();
    const volume = screen.getByLabelText(t.inspector.audio.volume, { selector: "input" });
    expect((volume as HTMLInputElement).disabled).toBe(true);
  });
});

describe("Audio section (F1.77, F1.78)", () => {
  it("shows the volume in percent with its dB readout", () => {
    show(["video"]);
    const volume = screen.getByLabelText(t.inspector.audio.volume, { selector: "input" });
    expect((volume as HTMLInputElement).value).toBe("50");
    expect(screen.getByText(t.inspector.audio.decibels("−6.0"))).toBeTruthy();
  });

  it("sends a typed volume as one commit, clamped to 400 %", async () => {
    show(["video", "fast"]);
    type(screen.getByLabelText(t.inspector.audio.volume, { selector: "input" }), "900");
    await settle();
    expect(calls).toEqual([
      [
        "setClipProperty",
        { clips: ["video", "fast"], value: { property: "volume", value: 4 }, phase: "commit" },
      ],
    ]);
  });

  it("clamps a typed fade to the room the other fade leaves", async () => {
    show(["video"]);
    type(screen.getByLabelText(t.inspector.audio.fadeIn, { selector: "input" }), "5");
    await settle();
    expect(calls).toEqual([
      ["setClipProperty", { clips: ["video"], value: { property: "fadeIn", value: S }, phase: "commit" }],
    ]);
  });

  it("steps the volume with the keyboard as one commit", async () => {
    show(["video"]);
    const thumb = screen.getByRole("slider", { name: t.inspector.audio.volume });
    fireEvent.keyDown(thumb, { key: "ArrowRight" });
    await settle();
    expect(calls).toEqual([
      ["setClipProperty", { clips: ["video"], value: { property: "volume", value: 0.51 }, phase: "commit" }],
    ]);
  });

  it("sends nothing when the typed value is unchanged or not a number, and Escape discards", async () => {
    show(["video"]);
    const volume = screen.getByLabelText(t.inspector.audio.volume, { selector: "input" });
    type(volume, "50");
    type(volume, "loud");
    fireEvent.change(volume, { target: { value: "80" } });
    fireEvent.keyDown(volume, { key: "Escape" });
    fireEvent.blur(volume);
    await settle();
    expect(calls).toEqual([]);
    expect((volume as HTMLInputElement).value).toBe("50");
  });
});

describe("Speed section (F1.79–F1.81)", () => {
  it("sends a preset as one commit", async () => {
    show(["video"]);
    fireEvent.click(screen.getByRole("button", { name: t.inspector.speed.presetLabel("2") }));
    await settle();
    expect(calls).toEqual([["setClipSpeed", { clips: ["video"], speed: 200, phase: "commit" }]]);
  });

  it("rounds and clamps a typed factor before sending it", async () => {
    show(["video"]);
    const field = screen.getByLabelText(t.inspector.speed.speed, { selector: "input" });
    type(field, "0.05");
    type(field, "1.234");
    await settle();
    expect(calls).toEqual([
      ["setClipSpeed", { clips: ["video"], speed: 10, phase: "commit" }],
      ["setClipSpeed", { clips: ["video"], speed: 123, phase: "commit" }],
    ]);
  });

  it("toggles reverse and preserve pitch with one command each", async () => {
    show(["video"]);
    fireEvent.click(screen.getByLabelText(t.inspector.speed.reverse));
    fireEvent.click(screen.getByLabelText(t.inspector.speed.preservePitch));
    await settle();
    expect(calls).toEqual([
      ["setClipReverse", { clips: ["video"], reverse: true }],
      [
        "setClipProperty",
        { clips: ["video"], value: { property: "preservePitch", value: false }, phase: "commit" },
      ],
    ]);
  });

  it("shows the resulting duration and the mute note above 4×", () => {
    show(["fast"]);
    const speed = section(t.inspector.speed.title);
    expect(speed.getByText("00:00:02:00")).toBeTruthy();
    expect(speed.getByText(t.inspector.speed.mutedAbove)).toBeTruthy();
    expect(
      speed.getByRole("button", { name: t.inspector.speed.presetLabel("1") }).getAttribute("aria-pressed"),
    ).toBe("false");
  });
});

describe("Duration section (F1.83)", () => {
  it("trims the end edge to the typed length, rounded to frames", async () => {
    show(["image"]);
    type(screen.getByLabelText(t.inspector.still.field, { selector: "input" }), "3.01");
    await settle();
    expect(calls).toEqual([
      ["trimClip", { clip: "image", edge: "end", time: 2 * S + 3 * S, phase: "commit" }],
    ]);
  });
});
