import { create } from "zustand";

import type { MediaKind } from "@/bindings";
import type { MediaSortKey, SortDirection } from "@/features/media-bin/browse";

export type MediaLayout = "grid" | "list";

/** The collapsible side panels that menu actions toggle. */
export type PanelKey = "mediaBin" | "inspector";

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
  preferencesOpen: boolean;
  /** Mirrors the panels' collapsed state (the panels own it), for the View menu check marks. */
  collapsedPanels: Readonly<Record<PanelKey, boolean>>;
  /** Changes when the panel layout is reset, so the panel groups remount with default sizes. */
  layoutVersion: number;
  setHistoryPanelOpen: (open: boolean) => void;
  setShortcutsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  setPreferencesOpen: (open: boolean) => void;
  setPanelCollapsed: (panel: PanelKey, collapsed: boolean) => void;
  bumpLayoutVersion: () => void;
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
  preferencesOpen: false,
  collapsedPanels: { mediaBin: false, inspector: false },
  layoutVersion: 0,
  setHistoryPanelOpen: (historyPanelOpen) => set({ historyPanelOpen }),
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  setPreferencesOpen: (preferencesOpen) => set({ preferencesOpen }),
  setPanelCollapsed: (panel, collapsed) =>
    set((s) =>
      s.collapsedPanels[panel] === collapsed
        ? s
        : { collapsedPanels: { ...s.collapsedPanels, [panel]: collapsed } },
    ),
  bumpLayoutVersion: () => set((s) => ({ layoutVersion: s.layoutVersion + 1 })),
  setMediaLayout: (mediaLayout) => set({ mediaLayout }),
  setMediaSort: (mediaSort, mediaSortDirection) => set({ mediaSort, mediaSortDirection }),
  setMediaKinds: (mediaKinds) => set({ mediaKinds }),
}));
