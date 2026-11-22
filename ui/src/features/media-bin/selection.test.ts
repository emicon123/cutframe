import { describe, expect, it } from "vitest";

import {
  emptySelection,
  liveSelection,
  type Selection,
  type SelectionAction,
  selectionReducer,
} from "./selection";

const order = ["a", "b", "c", "d", "e"];

function reduce(actions: SelectionAction[], state: Selection = emptySelection): Selection {
  return actions.reduce(selectionReducer, state);
}

const click = (id: string, mods: { ctrl?: boolean; shift?: boolean } = {}): SelectionAction => ({
  type: "click",
  id,
  ctrl: mods.ctrl ?? false,
  shift: mods.shift ?? false,
  order,
});

describe("selectionReducer", () => {
  it("selects one item on a plain click", () => {
    expect(reduce([click("b"), click("d")])).toEqual({ ids: ["d"], anchor: "d", focus: "d" });
  });

  it("toggles items with Ctrl-click", () => {
    expect(reduce([click("b"), click("d", { ctrl: true })]).ids).toEqual(["b", "d"]);
    expect(reduce([click("b"), click("d", { ctrl: true }), click("b", { ctrl: true })]).ids).toEqual(["d"]);
  });

  it("selects a range from the anchor with Shift-click, in either direction", () => {
    expect(reduce([click("b"), click("d", { shift: true })]).ids).toEqual(["b", "c", "d"]);
    const back = reduce([click("d"), click("a", { shift: true })]);
    expect(back).toEqual({ ids: ["a", "b", "c", "d"], anchor: "d", focus: "a" });
    // A second Shift-click replaces the range from the same anchor.
    expect(reduce([click("b"), click("e", { shift: true }), click("c", { shift: true })]).ids).toEqual([
      "b",
      "c",
    ]);
  });

  it("adds a range to the selection with Ctrl+Shift-click", () => {
    const state = reduce([click("a"), click("c", { ctrl: true }), click("e", { ctrl: true, shift: true })]);
    expect(state.ids).toEqual(["a", "c", "d", "e"]);
  });

  it("treats Shift-click without an anchor as a plain click", () => {
    expect(reduce([click("c", { shift: true })]).ids).toEqual(["c"]);
  });

  it("moves with arrow keys, clamped at the ends, and extends with Shift", () => {
    const move = (step: number, shift = false): SelectionAction => ({ type: "move", step, shift, order });
    expect(reduce([move(1)]).ids).toEqual(["a"]);
    expect(reduce([move(-1)]).ids).toEqual(["e"]);
    expect(reduce([click("b"), move(2)]).ids).toEqual(["d"]);
    expect(reduce([click("b"), move(10)]).ids).toEqual(["e"]);
    expect(reduce([click("b"), move(1, true), move(1, true)]).ids).toEqual(["b", "c", "d"]);
    expect(reduce([move(1)], { ...emptySelection }).focus).toBe("a");
    expect(selectionReducer(emptySelection, { type: "move", step: 1, shift: false, order: [] })).toBe(
      emptySelection,
    );
  });

  it("selects all and clears", () => {
    expect(reduce([click("c"), { type: "selectAll", order }]).ids).toEqual(order);
    expect(reduce([click("c"), { type: "clear" }])).toEqual(emptySelection);
  });

  it("keeps a selection that contains the right-clicked item, otherwise selects only it", () => {
    const two = reduce([click("a"), click("b", { ctrl: true })]);
    expect(selectionReducer(two, { type: "context", id: "a" }).ids).toEqual(["a", "b"]);
    expect(selectionReducer(two, { type: "context", id: "d" }).ids).toEqual(["d"]);
  });
});

describe("liveSelection", () => {
  it("drops ids that are gone or hidden and follows the display order", () => {
    const state = reduce([click("d"), click("x", { ctrl: true }), click("a", { ctrl: true })]);
    expect(liveSelection(state, order)).toEqual(["a", "d"]);
    expect(liveSelection(state, ["d", "c"])).toEqual(["d"]);
    expect(liveSelection(emptySelection, order)).toEqual([]);
  });
});
