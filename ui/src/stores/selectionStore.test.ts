import { afterEach, describe, expect, it } from "vitest";

import { type ProjectState, useProjectStore } from "./projectStore";
import { clearSelection, selectAll, selectionStore, setGapSelection, setSelection } from "./selectionStore";

const withClips = (revision: number, ids: string[]) =>
  ({
    revision,
    project: { tracks: [{ id: "t", locked: false, clips: ids.map((id) => ({ id })) }] },
  }) as unknown as ProjectState;

afterEach(() => {
  useProjectStore.setState({ state: null });
  clearSelection();
});

describe("selectionStore", () => {
  it("drops clips that no longer exist after a project change", () => {
    useProjectStore.getState().apply(withClips(1, ["a", "b"]));
    selectAll();
    expect(selectionStore.getState().clips).toEqual(["a", "b"]);
    useProjectStore.getState().apply(withClips(2, ["b"]));
    expect(selectionStore.getState().clips).toEqual(["b"]);
  });

  it("does not notify for an unchanged selection", () => {
    setSelection(["a"]);
    const before = selectionStore.getState();
    setSelection(["a"]);
    expect(selectionStore.getState()).toBe(before);
  });
});

describe("gap selection", () => {
  const withGap = (revision: number, start: number) =>
    ({
      revision,
      project: {
        tracks: [{ id: "t", kind: "video", locked: false, clips: [{ id: "a", start, duration: 10 }] }],
      },
    }) as unknown as ProjectState;

  it("replaces the clip selection and is replaced by it", () => {
    setSelection(["a"]);
    setGapSelection({ track: "t", at: 5 });
    expect(selectionStore.getState()).toEqual({ clips: [], gap: { track: "t", at: 5 } });
    setSelection(["a"]);
    expect(selectionStore.getState().gap).toBeNull();
  });

  it("is kept while the gap exists and dropped once it is closed", () => {
    useProjectStore.getState().apply(withGap(1, 100));
    setGapSelection({ track: "t", at: 50 });
    useProjectStore.getState().apply(withGap(2, 60));
    expect(selectionStore.getState().gap).toEqual({ track: "t", at: 50 });
    useProjectStore.getState().apply(withGap(3, 0));
    expect(selectionStore.getState().gap).toBeNull();
  });
});
