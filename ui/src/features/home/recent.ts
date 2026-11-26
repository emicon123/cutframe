import { create, type StoreApi, type UseBoundStore } from "zustand";

import { commands, type RecentProject_Serialize } from "@/bindings";
import { fileActions } from "@/features/project/fileActions";
import { t } from "@/i18n";
import { describeError, logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";

export type RecentProject = RecentProject_Serialize;

/** Everything the home screen's list touches, so its logic is testable without Tauri. */
export type RecentDeps = {
  list: () => Promise<RecentProject[]>;
  remove: (path: string) => Promise<unknown>;
  rename: (path: string, name: string) => Promise<unknown>;
  duplicate: (path: string, name: string) => Promise<unknown>;
  trash: (path: string) => Promise<unknown>;
  reveal: (path: string) => Promise<unknown>;
  /** Opens the project behind the unsaved prompt and shows the editor (`fileActions.openPath`). */
  openPath: (path: string) => Promise<void>;
  reportError: (error: unknown) => void;
  logError: (error: unknown) => void;
};

export type RecentState = {
  /** Newest first, as listed by Rust (never pruned: missing files stay until removed). */
  projects: RecentProject[];
  /** The first list arrived; until then the empty state is not shown. */
  loaded: boolean;
};

export type Recent = {
  store: UseBoundStore<StoreApi<RecentState>>;
  /** Fetches the list again. Every action below ends with it, whether it succeeded or not. */
  load: () => Promise<void>;
  open: (path: string) => Promise<void>;
  /** Resolves `null` on success, otherwise the translated text to show in the rename dialog. */
  rename: (path: string, name: string) => Promise<string | null>;
  duplicate: (project: RecentProject) => Promise<void>;
  /** Moves the file to the Recycle Bin. The caller has asked for confirmation. */
  trash: (path: string) => Promise<void>;
  reveal: (path: string) => Promise<void>;
  remove: (path: string) => Promise<void>;
};

/** A missing file (moved, deleted or unreachable) can only be removed from the list. */
export function isMissing(project: RecentProject): boolean {
  return project.modifiedMs === null;
}

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

/** The file's modification date in the user's locale, e.g. "Nov 22, 2026". */
export function formatModified(modifiedMs: number): string {
  return dateFormat.format(new Date(modifiedMs));
}

function isInvalidEdit(error: unknown): boolean {
  return typeof error === "object" && error !== null && "kind" in error && error.kind === "invalidEdit";
}

export function createRecent(deps: RecentDeps): Recent {
  const store = create<RecentState>()(() => ({ projects: [], loaded: false }));

  async function load(): Promise<void> {
    try {
      store.setState({ projects: await deps.list(), loaded: true });
    } catch (error) {
      // Keep what is shown; the error is shown as a toast.
      store.setState({ loaded: true });
      deps.reportError(error);
    }
  }

  /** Runs a file action, reports a failure, then relists (the list may have changed either way). */
  async function act(action: () => Promise<unknown>): Promise<void> {
    try {
      await action();
    } catch (error) {
      deps.reportError(error);
    }
    await load();
  }

  return {
    store,
    load,
    // `openPath` reports its own errors; a missing file then shows as missing after the relist.
    open: async (path) => {
      await deps.openPath(path);
      await load();
    },
    rename: async (path, name) => {
      let message: string | null = null;
      try {
        await deps.rename(path, name.trim());
      } catch (error) {
        deps.logError(error);
        message = isInvalidEdit(error) ? t.home.rename.conflict : describeError(error);
      }
      await load();
      return message;
    },
    duplicate: (project) => act(() => deps.duplicate(project.path, t.home.copyName(project.name))),
    trash: (path) => act(() => deps.trash(path)),
    reveal: (path) => act(() => deps.reveal(path)),
    remove: (path) => act(() => deps.remove(path)),
  };
}

/** The app's recent list, wired to Rust commands and the shared file actions. */
export const recent = createRecent({
  list: () => commands.listRecentProjects(),
  remove: (path) => commands.removeRecentProject(path),
  rename: (path, name) => commands.renameProjectFile(path, name),
  duplicate: (path, name) => commands.duplicateProjectFile(path, name),
  trash: (path) => commands.trashProjectFile(path),
  reveal: (path) => commands.revealProjectFile(path),
  openPath: (path) => fileActions.openPath(path),
  reportError: (error) => useAppStore.getState().reportError(error),
  logError,
});
