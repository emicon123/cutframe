import { create } from "zustand";

import type { MediaKind } from "@/bindings";
import type { MediaSortKey, SortDirection } from "@/features/media-bin/browse";

export type MediaLayout = "grid" | "list";

/** What fills the window: the home screen (recent projects) or the editor (design `home-screen.md` §7). */
export type View = "home" | "editor";

/** The collapsible side panels that menu actions toggle. */
export type PanelKey = "mediaBin" | "inspector";

const SNAPPING_KEY = "timeline.snapping.v1";
const SKIMMING_KEY = "timeline.skimming.v1";

/**
 * The timeline snapping and skimming toggles are per-user UI preferences in `localStorage`, like
 * the panel layout (design `preferences.md` §6): they have no meaning in Rust. Blocked storage
 * means "on".
 */
function readToggle(key: string): boolean {
  try {
    return window.localStorage.getItem(key) !== "off";
  } catch {
    return true;
  }
}

function writeToggle(key: string, on: boolean) {
  try {
    window.localStorage.setItem(key, on ? "on" : "off");
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
  /** Help → About (F0.10). */
  aboutOpen: boolean;
  /** Mirrors the panels' collapsed state (the panels own it), for the View menu check marks. */
  collapsedPanels: Readonly<Record<PanelKey, boolean>>;
  /** Changes when the panel layout is reset, so the panel groups remount with default sizes. */
  layoutVersion: number;
  /** Timeline snapping (F1.45); Alt held during a drag bypasses it. */
  snapping: boolean;
  setSnapping: (on: boolean) => void;
  /** Hover skimming on the timeline (F1.55): the preview shows the frame under the pointer. */
  skimming: boolean;
  setSkimming: (on: boolean) => void;
  setView: (view: View) => void;
  setStarted: () => void;
  setHistoryPanelOpen: (open: boolean) => void;
  setShortcutsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  setPreferencesOpen: (open: boolean) => void;
  setAboutOpen: (open: boolean) => void;
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
  aboutOpen: false,
  collapsedPanels: { mediaBin: false, inspector: false },
  layoutVersion: 0,
  snapping: readToggle(SNAPPING_KEY),
  setSnapping: (snapping) => {
    writeToggle(SNAPPING_KEY, snapping);
    set({ snapping });
  },
  skimming: readToggle(SKIMMING_KEY),
  setSkimming: (skimming) => {
    writeToggle(SKIMMING_KEY, skimming);
    set({ skimming });
  },
  setView: (view) => set({ view }),
  setStarted: () => set({ started: true }),
  setHistoryPanelOpen: (historyPanelOpen) => set({ historyPanelOpen }),
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  setPreferencesOpen: (preferencesOpen) => set({ preferencesOpen }),
  setAboutOpen: (aboutOpen) => set({ aboutOpen }),
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
