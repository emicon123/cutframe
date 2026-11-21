import { create } from "zustand";

import type { AppInfo_Serialize } from "@/bindings";
import { describeError } from "@/ipc/errors";

type AppStore = {
  info: AppInfo_Serialize | null;
  /** Last failed command, shown in the status bar. */
  lastError: string | null;
  setInfo: (info: AppInfo_Serialize) => void;
  reportError: (error: unknown) => void;
  clearError: () => void;
};

export const useAppStore = create<AppStore>()((set) => ({
  info: null,
  lastError: null,
  setInfo: (info) => set({ info }),
  reportError: (error) => set({ lastError: describeError(error) }),
  clearError: () => set({ lastError: null }),
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
