import type { HistoryState } from "@/bindings";
import { historyLabelText, t } from "@/i18n";

export type HistoryRow = {
  /** The `jumpToHistory` index that shows this row's state. */
  index: number;
  label: string;
  /** `current`: the state shown now; `applied`: older, undo reaches it; `undone`: redo reaches it. */
  status: "applied" | "current" | "undone";
};

/**
 * Rows of the history panel: the start of the history (index 0), then one row per entry. Row `i`
 * is the state after entry `i - 1`, so clicking it jumps to `i` applied entries.
 */
export function historyRows(history: Pick<HistoryState, "entries" | "current">): HistoryRow[] {
  const labels = [t.historyPanel.start, ...history.entries.map((e) => historyLabelText(e.label))];
  return labels.map((label, index) => ({
    index,
    label,
    status: index === history.current ? "current" : index < history.current ? "applied" : "undone",
  }));
}
