import { create } from "zustand";

import type { ProjectState_Serialize } from "@/bindings";

export type ProjectState = ProjectState_Serialize;

type ProjectStore = {
  /** Mirror of the Rust document. `null` until the first `getProject` answer arrives. */
  state: ProjectState | null;
  /** Accepts a state from Rust, ignoring anything older than what we already have. */
  apply: (next: ProjectState) => void;
};

export const useProjectStore = create<ProjectStore>()((set, get) => ({
  state: null,
  apply: (next) => {
    const current = get().state;
    if (current && next.revision < current.revision) return;
    set({ state: next });
  },
}));
