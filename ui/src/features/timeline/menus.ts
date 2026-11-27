/** What the timeline's right-click menu lists (F1.56): registry actions plus replace media. */
import type { ClipId } from "@/bindings";
import type { ActionId } from "@/features/editor/actions";

/** The clip that was right-clicked, or an empty lane spot. */
export type MenuTarget = { kind: "clip"; clip: ClipId } | { kind: "lane" };

export type MenuEntry = ActionId | "replace-media" | "-";

const CLIP_MENU: readonly MenuEntry[] = [
  "split",
  "freeze-frame",
  "reverse",
  "-",
  "cut",
  "copy",
  "paste",
  "duplicate",
  "-",
  "link-toggle",
  "detach-audio",
  "replace-media",
  "-",
  "delete",
  "ripple-delete",
];

const LANE_MENU: readonly MenuEntry[] = ["paste", "split", "-", "select-all"];

/** The entries of the menu for a right-click on a clip or on an empty lane spot. */
export function contextMenuItems(target: MenuTarget["kind"]): readonly MenuEntry[] {
  return target === "clip" ? CLIP_MENU : LANE_MENU;
}
