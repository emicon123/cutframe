import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { commands, PROJECT_FILE_EXTENSION } from "@/bindings";
import { exportRunning, stopExport } from "@/features/export/exportStore";
import { askCancelExport } from "@/features/export/quitPrompt";
import { type FailureContext, showErrorDialog } from "@/features/notifications/errorDialogStore";
import { t } from "@/i18n";
import { logError } from "@/ipc/errors";
import { safeFileStem } from "@/lib/fileName";
import { useAppStore } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore, type View } from "@/stores/uiStore";

import { askUnsaved, type UnsavedChoice } from "./unsavedPrompt";

/**
 * File name proposed by Save As: the project name with characters invalid on Windows replaced by
 * `_`, plus the project extension. Trailing dots and spaces are dropped (Windows strips them).
 */
export function defaultFileName(projectName: string): string {
  return `${safeFileStem(projectName, t.file.untitled)}.${PROJECT_FILE_EXTENSION}`;
}

/**
 * Default path for the Save As dialog: the proposed file name, in the folder of the current file
 * when the project already has one.
 */
export function defaultSavePath(projectName: string, filePath: string | null): string {
  const name = defaultFileName(projectName);
  if (!filePath) return name;
  const cut = Math.max(filePath.lastIndexOf("\\"), filePath.lastIndexOf("/"));
  return cut < 0 ? name : `${filePath.slice(0, cut + 1)}${name}`;
}

/** Window title for the current state: `*<name> - Cutframe` when dirty. */
export function formatWindowTitle(state: { name: string; dirty: boolean } | null): string {
  return state ? t.app.windowTitle(state.name, state.dirty) : t.app.name;
}

/** Everything the file actions touch, so the decision logic is testable without Tauri. */
export type FileActionDeps = {
  newProject: () => Promise<unknown>;
  openProject: (path: string) => Promise<unknown>;
  /** `false`: the project has no file yet and nothing was written. */
  saveProject: () => Promise<boolean>;
  saveProjectAs: (path: string) => Promise<unknown>;
  confirmClose: () => Promise<unknown>;
  /** The current project, or `null` before the first state arrived. */
  current: () => { name: string; filePath: string | null; dirty: boolean } | null;
  askUnsaved: () => Promise<UnsavedChoice>;
  /** An export is running (export.md §5.1). */
  exportRunning: () => boolean;
  /** "An export is running. Cancel it and quit?"; `true` to cancel it and quit. */
  askCancelExport: () => Promise<boolean>;
  /** Cancels the running export and waits for its `jobFinished` (at most 2 s). */
  stopExport: () => Promise<void>;
  /** Native open dialog; `null` when cancelled. */
  pickOpenPath: () => Promise<string | null>;
  /** Native save dialog; `null` when cancelled. */
  pickSavePath: (defaultPath: string) => Promise<string | null>;
  /** A failure that needs no decision: an error toast. */
  reportError: (error: unknown) => void;
  /** A failed open or save: the error dialog (F0.08). */
  showFailure: (context: FailureContext, error: unknown) => void;
  /** Switches between the home screen and the editor. */
  showView: (view: View) => void;
};

export type FileActions = {
  newProject: () => Promise<void>;
  openProject: () => Promise<void>;
  /**
   * Opens the project at `path` (a double-clicked file, `takeLaunchPath`/`openFileRequested`)
   * behind the unsaved-changes prompt; a failure shows the error dialog like File → Open. Ignored while another
   * file action or prompt is running.
   */
  openPath: (path: string) => Promise<void>;
  /** Resolves `true` when the project was written. */
  save: () => Promise<boolean>;
  saveAs: () => Promise<boolean>;
  /**
   * Answer to `quitRequested` and File → Exit: asks about a running export (`exporting` comes
   * from `quitRequested`; the export store is asked too) and about unsaved changes, then cancels
   * the export, waits for it to end and closes the window. Any "stay" answer keeps everything
   * running.
   */
  requestClose: (exporting?: boolean) => Promise<void>;
  /**
   * Replaces the open project with `action` behind the same unsaved-changes prompt as New/Open.
   * Resolves `true` when `action` ran; a rejection of `action` is passed on to the caller (who
   * shows it), not reported as a toast.
   */
  replaceGuarded: (action: () => Promise<unknown>) => Promise<boolean>;
  /**
   * File → Home: behind the unsaved-changes prompt, replaces the project with a fresh untitled one
   * (the model always holds one document) and shows the home screen. Cancel stays in the editor.
   */
  goHome: () => Promise<void>;
};

const FAILED: unique symbol = Symbol("failed");

export function createFileActions(deps: FileActionDeps): FileActions {
  /** One file action at a time: a second Ctrl+S or a close click during a prompt is ignored. */
  let busy = false;

  async function exclusive<T>(fallback: T, action: () => Promise<T>): Promise<T> {
    if (busy) return fallback;
    busy = true;
    try {
      return await action();
    } finally {
      busy = false;
    }
  }

  /**
   * Runs a step; a failure gives `FAILED` and is shown, as the error dialog for `context`
   * (a project write or read) or else as a toast.
   */
  async function attempt<T>(step: () => Promise<T>, context?: FailureContext): Promise<T | typeof FAILED> {
    try {
      return await step();
    } catch (error) {
      if (context) deps.showFailure(context, error);
      else deps.reportError(error);
      return FAILED;
    }
  }

  async function saveAs(): Promise<boolean> {
    const state = deps.current();
    if (!state) return false;
    const path = await attempt(() => deps.pickSavePath(defaultSavePath(state.name, state.filePath)));
    if (path === FAILED || path === null) return false;
    return (await attempt(() => deps.saveProjectAs(path), "save")) !== FAILED;
  }

  async function save(): Promise<boolean> {
    const written = await attempt(() => deps.saveProject(), "save");
    if (written === FAILED) return false;
    return written || saveAs();
  }

  /**
   * Runs `action` unless unsaved changes would be lost: Save (falls back to Save As; a cancelled or
   * failed save aborts), Don't save (runs it) or Cancel (does nothing).
   */
  async function guardUnsaved(action: () => Promise<unknown>): Promise<void> {
    if (await mayReplace()) await attempt(action);
  }

  /** Asks about unsaved changes; `true` when the open project may be replaced. */
  async function mayReplace(): Promise<boolean> {
    if (!deps.current()?.dirty) return true;
    const choice = await deps.askUnsaved();
    if (choice === "cancel") return false;
    return choice === "discard" || (await save());
  }

  /** Opens `path` and shows the editor once it succeeded; a failure shows the error dialog. */
  async function openInEditor(path: string): Promise<void> {
    if ((await attempt(() => deps.openProject(path), "open")) !== FAILED) deps.showView("editor");
  }

  return {
    newProject: () =>
      exclusive(undefined, () =>
        guardUnsaved(async () => {
          await deps.newProject();
          deps.showView("editor");
        }),
      ),
    openProject: () =>
      exclusive(undefined, () =>
        guardUnsaved(async () => {
          const path = await deps.pickOpenPath();
          if (path !== null) await openInEditor(path);
        }),
      ),
    openPath: (path) => exclusive(undefined, () => guardUnsaved(() => openInEditor(path))),
    save: () => exclusive(false, save),
    saveAs: () => exclusive(false, saveAs),
    requestClose: (exporting = false) =>
      exclusive(undefined, async () => {
        const running = exporting || deps.exportRunning();
        if (running && !(await deps.askCancelExport())) return;
        await guardUnsaved(async () => {
          if (running) await deps.stopExport();
          await deps.confirmClose();
        });
      }),
    replaceGuarded: (action) =>
      exclusive(false, async () => {
        if (!(await mayReplace())) return false;
        await action();
        return true;
      }),
    goHome: () =>
      exclusive(undefined, () =>
        guardUnsaved(async () => {
          await deps.newProject();
          deps.showView("home");
        }),
      ),
  };
}

const projectFilter = () => [{ name: t.file.filterName, extensions: [PROJECT_FILE_EXTENSION] }];

/** The app's file actions, wired to Rust commands and the native dialogs. */
export const fileActions: FileActions = createFileActions({
  newProject: () => commands.newProject(null),
  openProject: (path) => commands.openProject(path),
  saveProject: () => commands.saveProject(),
  saveProjectAs: (path) => commands.saveProjectAs(path),
  confirmClose: () => commands.confirmClose(),
  current: () => {
    const state = useProjectStore.getState().state;
    return state ? { name: state.project.name, filePath: state.filePath, dirty: state.dirty } : null;
  },
  askUnsaved,
  exportRunning,
  askCancelExport,
  stopExport: () => stopExport(),
  pickOpenPath: () => openDialog({ multiple: false, directory: false, filters: projectFilter() }),
  pickSavePath: (defaultPath) => saveDialog({ defaultPath, filters: projectFilter() }),
  reportError: (error) => useAppStore.getState().reportError(error),
  showFailure: (context, error) => {
    logError(error);
    showErrorDialog(context, error);
  },
  showView: (view) => useUiStore.getState().setView(view),
});
