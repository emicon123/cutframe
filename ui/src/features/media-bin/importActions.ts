import { commands, type JobId } from "@/bindings";
import { t } from "@/i18n";
import { run } from "@/stores/appStore";
import { useImportStore } from "@/stores/importStore";

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
  const { jobs } = useImportStore.getState();
  for (const job of jobs) if (!job.cancelling) cancelImport(job.id);
  return jobs.length > 0;
}
