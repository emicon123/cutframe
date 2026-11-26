import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { type IpcError, mockCommands } from "@/bindings";
import { useAppStore } from "@/stores/appStore";
import { playheadStore, setPlayhead } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { clearSelection, selectionStore, setGapSelection, setSelection } from "@/stores/selectionStore";

import {
  deleteRequest,
  deleteSelection,
  detachSelectedAudio,
  moveTrack,
  pasteAtPlayhead,
  playheadAfterTrimLeft,
  replaceClipMedia,
  setTrackFlag,
  splitAtPlayhead,
  toggleLinkSelection,
  trimToPlayhead,
} from "./editActions";

const empty: IpcError = { kind: "invalidEdit", message: "the clipboard is empty" };
const clip = (id: string, start: number, duration: number) => ({ id, start, duration });

describe("deleteRequest", () => {
  it("deletes the selected gap first, else the selected clips", () => {
    expect(deleteRequest({ clips: [], gap: { track: "t", at: 5 } }, true)).toEqual({
      kind: "gap",
      track: "t",
      at: 5,
    });
    expect(deleteRequest({ clips: ["a"], gap: null }, true)).toEqual({
      kind: "clips",
      clips: ["a"],
      ripple: true,
    });
    expect(deleteRequest({ clips: [], gap: null }, false)).toBeNull();
  });
});

describe("playheadAfterTrimLeft", () => {
  const project = (magnetic: boolean) => ({
    mainTrack: "main",
    magnetic,
    tracks: [
      { id: "main", kind: "video" as const, locked: false, clips: [clip("a", 0, 100), clip("b", 100, 100)] },
      { id: "over", kind: "video" as const, locked: false, clips: [clip("o", 0, 300)] },
    ],
  });

  it("moves to the start of the main-track clip under the playhead on a magnetic main track", () => {
    expect(playheadAfterTrimLeft(project(true), [], 150)).toBe(100);
    expect(playheadAfterTrimLeft(project(true), ["b"], 150)).toBe(100);
  });

  it("stays when the track is not magnetic, nothing on the main track is trimmed or at an edge", () => {
    expect(playheadAfterTrimLeft(project(false), [], 150)).toBeNull();
    expect(playheadAfterTrimLeft(project(true), ["o"], 150)).toBeNull();
    expect(playheadAfterTrimLeft(project(true), ["a"], 150)).toBeNull();
    expect(playheadAfterTrimLeft(project(true), [], 100)).toBeNull();
  });
});

describe("edit actions over IPC", () => {
  let calls: unknown[] = [];
  const state = {
    revision: 1,
    project: {
      mainTrack: "main",
      magnetic: true,
      tracks: [
        { id: "main", kind: "video", locked: false, clips: [clip("a", 0, 100), clip("b", 100, 100)] },
        { id: "over", kind: "video", locked: false, clips: [clip("o", 300, 100)] },
      ],
    },
  } as unknown as ProjectState;

  beforeEach(() => {
    calls = [];
    useProjectStore.setState({ state });
    mockCommands({
      splitClips: (args) => {
        calls.push(["split", args]);
        return ["a2"];
      },
      deleteGap: (args) => {
        calls.push(["deleteGap", args]);
        return null;
      },
      deleteClips: (args) => {
        calls.push(["deleteClips", args]);
        return null;
      },
      trimToPlayhead: (args) => {
        calls.push(["trim", args]);
        return null;
      },
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      pasteClips: () => Promise.reject(empty),
    });
  });

  afterEach(() => {
    useProjectStore.setState({ state: null });
    useAppStore.getState().clearError();
    clearSelection();
    setPlayhead(0);
  });

  it("splits under the playhead with no selection and selects the right halves", async () => {
    setPlayhead(50);
    await splitAtPlayhead();
    expect(calls).toEqual([["split", { clips: [], at: 50 }]]);
    expect(selectionStore.getState().clips).toEqual(["a2"]);
  });

  it("deletes a selected gap with deleteGap and clears it", async () => {
    setGapSelection({ track: "over", at: 250 });
    await deleteSelection(false);
    expect(calls).toEqual([["deleteGap", { track: "over", at: 250 }]]);
    expect(selectionStore.getState().gap).toBeNull();
  });

  it("ripple-deletes the selected clips", async () => {
    setSelection(["o"]);
    await deleteSelection(true);
    expect(calls).toEqual([["deleteClips", { clips: ["o"], ripple: true }]]);
  });

  it("moves the playhead to the trimmed clip's start after Q on the magnetic main track", async () => {
    setPlayhead(150);
    await trimToPlayhead("left");
    expect(calls).toEqual([["trim", { clips: [], at: 150, side: "left" }]]);
    expect(playheadStore.getState().time).toBe(100);
  });

  it("keeps the playhead after W", async () => {
    setPlayhead(150);
    await trimToPlayhead("right");
    expect(playheadStore.getState().time).toBe(150);
  });

  it("reports a failed paste and keeps the selection", async () => {
    setSelection(["a"]);
    await pasteAtPlayhead();
    expect(useAppStore.getState().lastError).not.toBeNull();
    expect(selectionStore.getState().clips).toEqual(["a"]);
  });
});

describe("link, detach and track actions over IPC", () => {
  let calls: unknown[] = [];
  const src = { kind: "media", media: "m", sourceIn: 0 };
  const state = {
    revision: 1,
    project: {
      mainTrack: "main",
      magnetic: true,
      tracks: [
        {
          id: "main",
          kind: "video",
          locked: false,
          clips: [
            { ...clip("v", 0, 100), source: src, link: null, audio: {} },
            { ...clip("w", 100, 100), source: src, link: "L", audio: null },
          ],
        },
        {
          id: "music",
          kind: "audio",
          locked: false,
          clips: [{ ...clip("m", 0, 100), source: src, link: "L" }],
        },
      ],
    },
  } as unknown as ProjectState;
  const record =
    <R>(name: string, result: R) =>
    (args: unknown): R => {
      calls.push([name, args]);
      return result;
    };

  beforeEach(() => {
    calls = [];
    useProjectStore.setState({ state });
    mockCommands({
      linkClips: record("link", null),
      unlinkClips: record("unlink", null),
      detachAudio: record("detach", ["new-audio"] as string[]),
      replaceClipMedia: record("replace", null),
      setTrackFlag: record("flag", null),
      moveTrack: record("moveTrack", null),
    });
  });

  afterEach(() => {
    useProjectStore.setState({ state: null });
    clearSelection();
  });

  it("unlinks the whole selection when it holds a linked clip", async () => {
    setSelection(["v", "m"]);
    await toggleLinkSelection();
    expect(calls).toEqual([["unlink", { clips: ["v", "m"] }]]);
  });

  it("links when no selected clip is linked", async () => {
    useProjectStore.setState({
      state: {
        ...state,
        project: {
          ...state.project,
          tracks: state.project.tracks.map((tr) => ({
            ...tr,
            clips: tr.clips.map((c) => ({ ...c, link: null })),
          })),
        },
      },
    });
    setSelection(["v", "m"]);
    await toggleLinkSelection();
    expect(calls).toEqual([["link", { clips: ["v", "m"] }]]);
  });

  it("sends nothing when the toggle does not apply", async () => {
    setSelection(["v"]);
    await toggleLinkSelection();
    expect(calls).toEqual([]);
  });

  it("detaches audio and selects the new audio clips", async () => {
    setSelection(["v"]);
    await detachSelectedAudio();
    expect(calls).toEqual([["detach", { clips: ["v"] }]]);
    expect(selectionStore.getState().clips).toEqual(["new-audio"]);
  });

  it("replaces media, sets track flags and moves tracks", async () => {
    expect(await replaceClipMedia("v", "other")).toBe(true);
    await setTrackFlag("music", "muted", true);
    await moveTrack("music", 2);
    expect(calls).toEqual([
      ["replace", { clip: "v", media: "other" }],
      ["flag", { track: "music", flag: "muted", value: true }],
      ["moveTrack", { track: "music", index: 2 }],
    ]);
  });
});
