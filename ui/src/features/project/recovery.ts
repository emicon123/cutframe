import { create, type StoreApi, type UseBoundStore } from "zustand";

import { commands, type RecoveryEntry_Serialize } from "@/bindings";
import { describeError, logError } from "@/ipc/errors";

import { fileActions } from "./fileActions";

export type RecoveryEntry = RecoveryEntry_Serialize;

/** Everything the recovery prompt touches, so its logic is testable without Tauri. */
export type RecoveryDeps = {
  listRecovery: () => Promise<RecoveryEntry[]>;
  restoreRecovery: (id: string) => Promise<unknown>;
  discardRecovery: (id: string) => Promise<unknown>;
  /** Runs the action behind the unsaved-changes prompt; `true` when it ran (`replaceGuarded`). */
  replaceGuarded: (action: () => Promise<unknown>) => Promise<boolean>;
  logError: (error: unknown) => void;
};

export type RecoveryState = {
  /** Entries still offered, newest first. The dialog is open while this is not empty. */
  entries: RecoveryEntry[];
  /** Translated text of the last failed restore or discard, shown inside the dialog. */
  error: string | null;
  /** A restore or discard is running; the buttons are disabled. */
  busy: boolean;
};

export type Recovery = {
  store: UseBoundStore<StoreApi<RecoveryState>>;
  /** Fetches the entries. Only the first call does anything (once per webview load). */
  load: () => Promise<void>;
  /** Restores `id` after the unsaved prompt and closes the dialog; other entries stay on disk. */
  restore: (id: string) => Promise<void>;
  /** Deletes `id` and removes its row. */
  discard: (id: string) => Promise<void>;
  /** Closes the dialog and keeps every file for the next start. */
  later: () => void;
};

/** Entries sorted newest first (the backend already does this; the UI does not rely on it). */
export function sortEntries(entries: readonly RecoveryEntry[]): RecoveryEntry[] {
  return [...entries].sort((a, b) => b.savedAtMs - a.savedAtMs);
}

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** The autosave time in the user's locale, e.g. "Nov 22, 2026, 3:04 PM". */
export function formatSavedAt(savedAtMs: number): string {
  return timeFormat.format(new Date(savedAtMs));
}

export function createRecovery(deps: RecoveryDeps): Recovery {
  const store = create<RecoveryState>()(() => ({ entries: [], error: null, busy: false }));
  let loaded = false;

  /** Runs one restore/discard at a time; a failure is logged and shown in the dialog. */
  async function step(action: () => Promise<void>): Promise<void> {
    if (store.getState().busy) return;
    store.setState({ busy: true });
    try {
      await action();
      store.setState({ error: null });
    } catch (error) {
      deps.logError(error);
      store.setState({ error: describeError(error) });
    } finally {
      store.setState({ busy: false });
    }
  }

  return {
    store,
    load: async () => {
      if (loaded) return;
      loaded = true;
      try {
        store.setState({ entries: sortEntries(await deps.listRecovery()), error: null });
      } catch (error) {
        // Nothing to offer is the safe default; the files stay for the next start.
        deps.logError(error);
      }
    },
    restore: (id) =>
      step(async () => {
        const ran = await deps.replaceGuarded(() => deps.restoreRecovery(id));
        if (ran) store.setState({ entries: [] });
      }),
    discard: (id) =>
      step(async () => {
        await deps.discardRecovery(id);
        store.setState((s) => ({ entries: s.entries.filter((entry) => entry.id !== id) }));
      }),
    later: () => {
      if (!store.getState().busy) store.setState({ entries: [], error: null });
    },
  };
}

/** The app's recovery prompt, wired to Rust commands and the shared file actions. */
export const recovery = createRecovery({
  listRecovery: () => commands.listRecovery(),
  restoreRecovery: (id) => commands.restoreRecovery(id),
  discardRecovery: (id) => commands.discardRecovery(id),
  replaceGuarded: (action) => fileActions.replaceGuarded(action),
  logError,
});
