import { commands, type JobId, type MediaId } from "@/bindings";
import { t } from "@/i18n";
import { run } from "@/stores/appStore";
import { isBackgroundJob, useImportStore } from "@/stores/importStore";

function track(id: JobId | null | undefined): void {
  if (id) useImportStore.getState().dispatch({ type: "started", id });
}

/** Opens the native file dialog (from Rust) and imports the chosen files. */
export async function importFromDialog(): Promise<void> {
  track(await run(commands.importMediaDialog(t.mediaBin.filterLabel)));
}

/** Imports dropped files and folders; Rust decides what is importable. */
export async function importPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  track(await run(commands.importMedia(paths)));
}

export function cancelImport(id: JobId): void {
  useImportStore.getState().dispatch({ type: "cancelRequested", id });
  void run(commands.cancelJob(id));
}

/** Cancels every running import (Esc in the media panel). Returns whether there was one. */
export function cancelAllImports(): boolean {
  const imports = useImportStore.getState().jobs.filter((job) => !isBackgroundJob(job));
  for (const job of imports) if (!job.cancelling) cancelImport(job.id);
  return imports.length > 0;
}

/** Removes media from the project (one undo step). Resolves whether it succeeded. */
export async function removeMedia(ids: readonly MediaId[]): Promise<boolean> {
  if (ids.length === 0) return false;
  return (await run(commands.removeMedia([...ids]))) !== undefined;
}

/** Opens Explorer with the media file selected. */
export function revealMedia(id: MediaId): void {
  void run(commands.revealMedia(id));
}
