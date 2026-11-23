import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type HistoryState, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";

import { historyRows } from "./history";
import { HistoryPanel } from "./HistoryPanel";

const history: Pick<HistoryState, "entries" | "current"> = {
  entries: [{ label: "addTrack" }, { label: "renameTrack" }, { label: "removeTrack" }],
  current: 2,
};

describe("historyRows", () => {
  it("starts with the start row and marks applied, current and undone entries", () => {
    expect(historyRows(history)).toEqual([
      { index: 0, label: t.historyPanel.start, status: "applied" },
      { index: 1, label: t.history.labels.addTrack, status: "applied" },
      { index: 2, label: t.history.labels.renameTrack, status: "current" },
      { index: 3, label: t.history.labels.removeTrack, status: "undone" },
    ]);
  });

  it("marks the start row current when everything is undone", () => {
    expect(historyRows({ ...history, current: 0 }).map((r) => r.status)).toEqual([
      "current",
      "undone",
      "undone",
      "undone",
    ]);
  });

  it("has only the start row for an empty history", () => {
    expect(historyRows({ entries: [], current: 0 })).toEqual([
      { index: 0, label: t.historyPanel.start, status: "current" },
    ]);
  });
});

const state = (h: Pick<HistoryState, "entries" | "current">): ProjectState => ({
  revision: 1,
  project: {
    id: "p",
    name: "Trip",
    settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
    duration: 0,
    background: { kind: "color", color: "#000000ff" },
    mainTrack: "main",
    magnetic: true,
    tracks: [],
    media: [],
  },
  history: {
    canUndo: h.current > 0,
    canRedo: h.current < h.entries.length,
    undoLabel: null,
    redoLabel: null,
    ...h,
  },
  filePath: null,
  dirty: false,
});

afterEach(() => {
  useProjectStore.setState({ state: null });
  useAppStore.setState({ lastError: null });
  vi.restoreAllMocks();
});

describe("HistoryPanel", () => {
  it("jumps to the clicked entry", async () => {
    const jumps: number[] = [];
    mockCommands({ jumpToHistory: ({ index }) => (jumps.push(index), true) });
    useProjectStore.setState({ state: state(history) });
    render(<HistoryPanel />);

    const current = screen.getByRole("button", { name: t.history.labels.renameTrack });
    expect(current.getAttribute("aria-current")).toBe("step");
    fireEvent.click(current);
    fireEvent.click(screen.getByRole("button", { name: t.historyPanel.start }));
    fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${t.history.labels.removeTrack}`) }));

    await waitFor(() => expect(jumps).toEqual([0, 3]));
  });

  it("reports a failed jump in the status bar", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockCommands({
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      jumpToHistory: () => Promise.reject({ kind: "invalidEdit", message: "out of range" }),
    });
    useProjectStore.setState({ state: state(history) });
    render(<HistoryPanel />);

    fireEvent.click(screen.getByRole("button", { name: t.history.labels.addTrack }));
    await waitFor(() => expect(useAppStore.getState().lastError).toBe(t.errors.kinds.invalidEdit));
  });
});
