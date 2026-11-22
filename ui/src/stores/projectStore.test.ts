import { afterEach, describe, expect, it } from "vitest";

import { type ProjectState, useProjectStore } from "./projectStore";

function stateAt(revision: number, name = `rev ${revision}`): ProjectState {
  return {
    revision,
    project: {
      id: "p",
      name,
      settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
      duration: 0,
      tracks: [],
    },
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
    filePath: null,
    dirty: false,
  };
}

afterEach(() => useProjectStore.setState({ state: null }));

describe("projectStore.apply", () => {
  it("keeps the payload with the highest revision", () => {
    const { apply } = useProjectStore.getState();
    apply(stateAt(5));
    apply(stateAt(7));
    apply(stateAt(6));
    expect(useProjectStore.getState().state?.revision).toBe(7);
  });

  it("accepts a newer revision after a project replacement", () => {
    // The revision is monotonic for the app session (ADR-0003 §6), so a new project continues it.
    const { apply } = useProjectStore.getState();
    apply(stateAt(12, "old project"));
    apply(stateAt(13, "new project"));
    expect(useProjectStore.getState().state?.project.name).toBe("new project");
  });
});
