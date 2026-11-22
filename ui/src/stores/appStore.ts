import { create } from "zustand";

import type { AppInfo_Serialize } from "@/bindings";
import { describeError, logError } from "@/ipc/errors";

type AppStore = {
  info: AppInfo_Serialize | null;
  /** Translated text for the last failed command, shown in the status bar. */
  lastError: string | null;
  /**
   * Number of open DOM overlays (menus, dialogs) that would be covered by the native preview
   * window (ADR-0005). The preview is hidden while it is above zero.
   */
  previewOccluders: number;
  setInfo: (info: AppInfo_Serialize) => void;
  reportError: (error: unknown) => void;
  clearError: () => void;
  addPreviewOccluder: () => void;
  removePreviewOccluder: () => void;
};

export const useAppStore = create<AppStore>()((set) => ({
  info: null,
  lastError: null,
  setInfo: (info) => set({ info }),
  reportError: (error) => {
    logError(error);
    set({ lastError: describeError(error) });
  },
  clearError: () => set({ lastError: null }),
  previewOccluders: 0,
  addPreviewOccluder: () => set((s) => ({ previewOccluders: s.previewOccluders + 1 })),
  removePreviewOccluder: () => set((s) => ({ previewOccluders: Math.max(0, s.previewOccluders - 1) })),
}));

/** Runs a command and reports a rejection instead of throwing. */
export async function run<T>(command: Promise<T>): Promise<T | undefined> {
  try {
    const result = await command;
    useAppStore.getState().clearError();
    return result;
  } catch (error) {
    useAppStore.getState().reportError(error);
    return undefined;
  }
}
