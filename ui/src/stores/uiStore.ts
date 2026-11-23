import { create } from "zustand";

import type { MediaKind } from "@/bindings";
import type { MediaSortKey, SortDirection } from "@/features/media-bin/browse";

export type MediaLayout = "grid" | "list";

/** What fills the window: the home screen (recent projects) or the editor (design `home-screen.md` §7). */
export type View = "home" | "editor";

/** The collapsible side panels that menu actions toggle. */
export type PanelKey = "mediaBin" | "inspector";

const SNAPPING_KEY = "timeline.snapping.v1";

/**
 * The timeline snapping toggle is a per-user UI preference in `localStorage`, like the panel
 * layout (design `preferences.md` §6): it has no meaning in Rust. Blocked storage means "on".
 */
function readSnapping(): boolean {
  try {
    return window.localStorage.getItem(SNAPPING_KEY) !== "off";
  } catch {
    return true;
  }
}

function writeSnapping(on: boolean) {
  try {
    window.localStorage.setItem(SNAPPING_KEY, on ? "on" : "off");
  } catch {
    // Blocked storage: the choice lasts for this session only.
  }
}

/** Editor layout and dialog toggles opened from the main menu or a shortcut. Not part of the model. */
type UiStore = {
  view: View;
  /**
   * `false` until startup has fetched the first state and opened the launch path (if any), so a
   * project double-clicked in Explorer opens without the home screen flashing first.
   */
  started: boolean;
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
  /** Timeline snapping (F1.45); Alt held during a drag bypasses it. */
  snapping: boolean;
  setSnapping: (on: boolean) => void;
  setView: (view: View) => void;
  setStarted: () => void;
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
  view: "home",
  started: false,
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
  snapping: readSnapping(),
  setSnapping: (snapping) => {
    writeSnapping(snapping);
    set({ snapping });
  },
  setView: (view) => set({ view }),
  setStarted: () => set({ started: true }),
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
