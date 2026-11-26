import { create } from "zustand";

import { commands, events } from "@/bindings";
import { fileActions } from "@/features/project/fileActions";
import { t } from "@/i18n";
import { describeError, errorToast, isIpcError, logError } from "@/ipc/errors";
import { showToast } from "@/stores/toastStore";

import { INITIAL_UPDATE_STATE, reduceUpdate, type UpdateEvent, type UpdateState } from "./updateState";

type UpdateStore = UpdateState & {
  /** The startup check ran (once per app session, also under React StrictMode). */
  startupChecked: boolean;
  /** `restartApp` is running (after the unsaved-changes prompt); the button is disabled. */
  restarting: boolean;
};

/** The update prompt's state (F0.12), rendered by `UpdatePrompt`. */
export const useUpdateStore = create<UpdateStore>()(() => ({
  ...INITIAL_UPDATE_STATE,
  startupChecked: false,
  restarting: false,
}));

function dispatch(event: UpdateEvent): void {
  useUpdateStore.setState((s) => reduceUpdate(s, event));
}

/**
 * Text for an update failure: `unavailable` means the server could not be reached or this build
 * does not check (debug without `CUTFRAME_UPDATER=1`), which the generic text does not say.
 */
export function updateErrorText(error: unknown): string {
  return isIpcError(error) && error.kind === "unavailable" ? t.updater.unreachable : describeError(error);
}

/**
 * Asks Rust for a newer release. The automatic check (`manual: false`) is silent: a failure only
 * reaches the console. Help → Check for updates… says "You're up to date" or shows the failure.
 * A found update shows the prompt either way. Ignored while a check is running.
 */
export async function checkForUpdates({ manual }: { manual: boolean }): Promise<void> {
  if (useUpdateStore.getState().checking) return;
  dispatch({ type: "checkStarted" });
  try {
    const info = await commands.checkForUpdate();
    dispatch({ type: "checkFinished", info });
    if (manual && info === null) showToast({ tone: "success", text: t.updater.upToDate });
  } catch (error) {
    dispatch({ type: "checkFailed" });
    logError(error);
    if (manual) showToast({ ...errorToast(error, t.updater.checkFailed), detail: updateErrorText(error) });
  }
}

/** The automatic check after startup; runs once per app session. */
export function checkOnStartup(): void {
  if (useUpdateStore.getState().startupChecked) return;
  useUpdateStore.setState({ startupChecked: true });
  void checkForUpdates({ manual: false });
}

/** Update / Retry: starts the download; progress arrives as `update-progress`. */
export async function installUpdate(): Promise<void> {
  dispatch({ type: "installStarted" });
  try {
    await commands.installUpdate();
  } catch (error) {
    logError(error);
    dispatch({
      type: "installFailed",
      error: isIpcError(error) ? error : { kind: "internal", message: String(error) },
    });
  }
}

/** "Later": hides the prompt; the download (if any) goes on. */
export function hideUpdatePrompt(): void {
  dispatch({ type: "hide" });
}

/**
 * Restart to update: the unsaved-changes prompt first (`restartApp` does not ask), then Rust runs
 * the installer and exits. Cancel, or a failed save, keeps the app running with the update ready.
 */
export async function restartToUpdate(): Promise<void> {
  if (useUpdateStore.getState().restarting) return;
  useUpdateStore.setState({ restarting: true });
  try {
    await fileActions.replaceGuarded(() => commands.restartApp());
  } catch (error) {
    logError(error);
    showToast(errorToast(error, t.updater.restartFailed));
  } finally {
    useUpdateStore.setState({ restarting: false });
  }
}

/** Feeds `update-progress` into the store. Start it before any download can start; returns a cleanup. */
export function startUpdateSync(): () => void {
  const subscription = events.updateProgress.listen((e) => dispatch({ type: "progress", event: e.payload }));
  subscription.catch(logError);
  return () => {
    void subscription.then((stop) => stop()).catch(() => undefined);
  };
}
