import { create } from "zustand";

import type { MediaKind } from "@/bindings";
import type { MediaSortKey, SortDirection } from "@/features/media-bin/browse";

export type MediaLayout = "grid" | "list";

/** Editor layout and dialog toggles opened from the main menu or a shortcut. Not part of the model. */
type UiStore = {
  historyPanelOpen: boolean;
  shortcutsOpen: boolean;
  projectSettingsOpen: boolean;
  /** Media panel view: grid of thumbnails or a dense list. */
  mediaLayout: MediaLayout;
  mediaSort: MediaSortKey;
  mediaSortDirection: SortDirection;
  /** Kinds the media panel shows; empty shows all. */
  mediaKinds: MediaKind[];
  setHistoryPanelOpen: (open: boolean) => void;
  setShortcutsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  setMediaLayout: (layout: MediaLayout) => void;
  setMediaSort: (sort: MediaSortKey, direction: SortDirection) => void;
  setMediaKinds: (kinds: MediaKind[]) => void;
};

export const useUiStore = create<UiStore>()((set) => ({
  historyPanelOpen: false,
  shortcutsOpen: false,
  projectSettingsOpen: false,
  mediaLayout: "grid",
  mediaSort: "imported",
  mediaSortDirection: "asc",
  mediaKinds: [],
  setHistoryPanelOpen: (historyPanelOpen) => set({ historyPanelOpen }),
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  setMediaLayout: (mediaLayout) => set({ mediaLayout }),
  setMediaSort: (mediaSort, mediaSortDirection) => set({ mediaSort, mediaSortDirection }),
  setMediaKinds: (mediaKinds) => set({ mediaKinds }),
}));
