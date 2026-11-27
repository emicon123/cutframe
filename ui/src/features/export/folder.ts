import { videoDir } from "@tauri-apps/api/path";

import { logError } from "@/ipc/errors";

const LAST_FOLDER_KEY = "export.lastFolder.v1";

/**
 * The folder of the last started export. A per-user UI preference in `localStorage`, like the
 * panel layout (export.md §2: not project data); blocked storage just forgets it.
 */
export function readLastFolder(): string | null {
  try {
    const folder = window.localStorage.getItem(LAST_FOLDER_KEY);
    return folder?.trim() ? folder : null;
  } catch {
    return null;
  }
}

export function rememberFolder(folder: string): void {
  try {
    window.localStorage.setItem(LAST_FOLDER_KEY, folder.trim());
  } catch {
    // Blocked storage: the folder is remembered for nothing longer than this dialog.
  }
}

/** The folder a new export dialog starts in: the last one, else the user's Videos folder, else none. */
export async function defaultExportFolder(): Promise<string> {
  const last = readLastFolder();
  if (last !== null) return last;
  try {
    return await videoDir();
  } catch (error) {
    logError(error);
    return "";
  }
}
