import type { MediaId } from "@/bindings";

/**
 * Selection in the media panel (UI state, not model). `ids` keeps the order items were selected;
 * `anchor` is where a Shift range starts and `focus` the item keyboard navigation moves from.
 */
export type Selection = {
  ids: readonly MediaId[];
  anchor: MediaId | null;
  focus: MediaId | null;
};

export const emptySelection: Selection = { ids: [], anchor: null, focus: null };

export type SelectionAction =
  /** A click (or arrow key) on `id`; `order` is the visible order, for Shift ranges. */
  | { type: "click"; id: MediaId; ctrl: boolean; shift: boolean; order: readonly MediaId[] }
  /** Arrow keys: move the focus by `step` in `order`, selecting it (Shift extends the range). */
  | { type: "move"; step: number; shift: boolean; order: readonly MediaId[] }
  | { type: "selectAll"; order: readonly MediaId[] }
  /** Right-click: keeps a selection that contains `id`, otherwise selects only `id`. */
  | { type: "context"; id: MediaId }
  | { type: "clear" };

function range(order: readonly MediaId[], from: MediaId, to: MediaId): MediaId[] | null {
  const a = order.indexOf(from);
  const b = order.indexOf(to);
  if (a < 0 || b < 0) return null;
  return order.slice(Math.min(a, b), Math.max(a, b) + 1);
}

export function selectionReducer(state: Selection, action: SelectionAction): Selection {
  switch (action.type) {
    case "click": {
      const { id, ctrl, shift, order } = action;
      if (shift && state.anchor !== null) {
        const ids = range(order, state.anchor, id);
        if (ids) {
          const merged = ctrl ? [...state.ids.filter((s) => !ids.includes(s)), ...ids] : ids;
          return { ids: merged, anchor: state.anchor, focus: id };
        }
      }
      if (ctrl) {
        const ids = state.ids.includes(id) ? state.ids.filter((s) => s !== id) : [...state.ids, id];
        return { ids, anchor: id, focus: id };
      }
      return { ids: [id], anchor: id, focus: id };
    }
    case "move": {
      const { order, step, shift } = action;
      if (order.length === 0) return state;
      const current = state.focus === null ? -1 : order.indexOf(state.focus);
      const start = current < 0 ? (step > 0 ? -1 : order.length) : current;
      const index = Math.min(order.length - 1, Math.max(0, start + step));
      const id = order[index];
      if (id === undefined) return state;
      return selectionReducer(state, { type: "click", id, ctrl: false, shift, order });
    }
    case "selectAll":
      return { ids: [...action.order], anchor: state.anchor, focus: state.focus };
    case "context":
      return state.ids.includes(action.id)
        ? { ...state, focus: action.id }
        : { ids: [action.id], anchor: action.id, focus: action.id };
    case "clear":
      return emptySelection;
  }
}

/**
 * The selected ids that still exist, in `order` (display order). Items disappear from the project
 * without a selection change (undo of an import, removal), so readers always go through this.
 */
export function liveSelection(selection: Selection, order: readonly MediaId[]): MediaId[] {
  if (selection.ids.length === 0) return [];
  const selected = new Set(selection.ids);
  return order.filter((id) => selected.has(id));
}
