import { afterEach, describe, expect, it } from "vitest";

import { type ProjectState, useProjectStore } from "./projectStore";
import { clearSelection, selectAll, selectionStore, setSelection } from "./selectionStore";

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
