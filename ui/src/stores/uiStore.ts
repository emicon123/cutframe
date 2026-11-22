import { create } from "zustand";

/** The collapsible side panels that menu actions toggle. */
export type PanelKey = "mediaBin" | "inspector";

/** Editor layout and dialog toggles opened from the main menu or a shortcut. Not part of the model. */
type UiStore = {
  historyPanelOpen: boolean;
  shortcutsOpen: boolean;
  projectSettingsOpen: boolean;
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
};

export const useUiStore = create<UiStore>()((set) => ({
  historyPanelOpen: false,
  shortcutsOpen: false,
  projectSettingsOpen: false,
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
}));
