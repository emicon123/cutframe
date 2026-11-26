import { create } from "zustand";

import type { AppInfo_Serialize } from "@/bindings";
import type { GpuNotice } from "@/features/editor/gpuNotice";
import { errorToast, logError } from "@/ipc/errors";

import { showToast } from "./toastStore";

type AppStore = {
  info: AppInfo_Serialize | null;
  /**
   * Number of open DOM overlays (menus, dialogs) that would be covered by the native preview
   * window (ADR-0005). The preview is hidden while it is above zero.
   */
  previewOccluders: number;
  /** Runtime GPU event shown in the status bar until dismissed. */
  gpuNotice: GpuNotice | null;
  setInfo: (info: AppInfo_Serialize) => void;
  /** Logs a failure and shows it as an error toast with a readable, translated message (F0.08). */
  reportError: (error: unknown) => void;
  showGpuNotice: (notice: GpuNotice) => void;
  dismissGpuNotice: () => void;
  addPreviewOccluder: () => void;
  removePreviewOccluder: () => void;
};

export const useAppStore = create<AppStore>()((set) => ({
  info: null,
  setInfo: (info) => set({ info }),
  reportError: (error) => {
    logError(error);
    showToast(errorToast(error));
  },
  gpuNotice: null,
  showGpuNotice: (gpuNotice) => set({ gpuNotice }),
  dismissGpuNotice: () => set({ gpuNotice: null }),
  previewOccluders: 0,
  addPreviewOccluder: () => set((s) => ({ previewOccluders: s.previewOccluders + 1 })),
  removePreviewOccluder: () => set((s) => ({ previewOccluders: Math.max(0, s.previewOccluders - 1) })),
}));

/** Runs a command and reports a rejection (as an error toast) instead of throwing. */
export async function run<T>(command: Promise<T>): Promise<T | undefined> {
  try {
    return await command;
  } catch (error) {
    useAppStore.getState().reportError(error);
    return undefined;
  }
}
