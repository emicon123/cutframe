import { create } from "zustand";

/** Editor layout and dialog toggles opened from the main menu or a shortcut. Not part of the model. */
type UiStore = {
  historyPanelOpen: boolean;
  shortcutsOpen: boolean;
  projectSettingsOpen: boolean;
  setHistoryPanelOpen: (open: boolean) => void;
  setShortcutsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
};

export const useUiStore = create<UiStore>()((set) => ({
  historyPanelOpen: false,
  shortcutsOpen: false,
  projectSettingsOpen: false,
  setHistoryPanelOpen: (historyPanelOpen) => set({ historyPanelOpen }),
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
}));
