import { describe, expect, it } from "vitest";

import { ACTIONS } from "@/features/editor/actions";

import { contextMenuItems } from "./menus";

describe("contextMenuItems", () => {
  it("lists the clip edits, link, detach and replace for a clip", () => {
    const items = contextMenuItems("clip");
    for (const id of [
      "split",
      "delete",
      "ripple-delete",
      "copy",
      "cut",
      "duplicate",
      "link-toggle",
      "detach-audio",
    ])
      expect(items).toContain(id);
    expect(items).toContain("replace-media");
  });

  it("offers paste on an empty lane, without clip-only entries", () => {
    const items = contextMenuItems("lane");
    expect(items).toContain("paste");
    expect(items).not.toContain("replace-media");
    expect(items).not.toContain("delete");
  });

  it("names only registered actions", () => {
    const ids = new Set<string>(ACTIONS.map((a) => a.id));
    for (const entry of [...contextMenuItems("clip"), ...contextMenuItems("lane")])
      if (entry !== "-" && entry !== "replace-media") expect(ids.has(entry)).toBe(true);
  });
});
