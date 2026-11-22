import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";

import { commands, type JobId, MEDIA_EXTENSIONS, type MediaId } from "@/bindings";
import { t } from "@/i18n";
import { waitForJob } from "@/ipc/jobResults";
import { run } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";

import { parentDirOf, type RelinkResult, summarizeRelink } from "./relink";

/**
 * State of the relink dialog (UI only: what is relinked lives in Rust and comes back through
 * `project-changed`, including each item's `status`).
 */
type RelinkState = {
  open: boolean;
  /** The item the dialog was opened for; `null` lists every item that needs a relink. */
  focus: MediaId | null;
  /** The relink job in flight; `jobId` is `null` until its command resolves. */
  running: { jobId: JobId | null; mode: "file" | "folder" } | null;
  /** A file with other content was chosen for `id`: ask before relinking with `allowMismatch`. */
  mismatch: { id: MediaId; path: string } | null;
  result: RelinkResult | null;
};

const closed: RelinkState = { open: false, focus: null, running: null, mismatch: null, result: null };

export const useRelinkStore = create<RelinkState>()(() => closed);

const set = (patch: Partial<RelinkState>) => useRelinkStore.setState(patch);

/** Each start bumps this, so the result of a job from an earlier dialog session is dropped. */
let session = 0;

export function openRelink(focus: MediaId | null): void {
  session += 1;
  useRelinkStore.setState({ ...closed, open: true, focus });
}

export function closeRelink(): void {
  session += 1;
  useRelinkStore.setState(closed);
}

async function runJob(mode: "file" | "folder", start: () => Promise<JobId | undefined>, path?: string) {
  const mine = ++session;
  set({ running: { jobId: null, mode }, mismatch: null, result: null });
  const jobId = await start();
  if (mine !== session) return;
  if (jobId === undefined) {
    set({ running: null });
    return;
  }
  set({ running: { jobId, mode } });
  const result = summarizeRelink(await waitForJob(jobId));
  if (mine !== session) return;
  const mismatch = result.mismatch !== null && path !== undefined ? { id: result.mismatch, path } : null;
  set({ running: null, result, mismatch });
}

/** Relinks `id` to `path`; with `allowMismatch` also to a file with other content. */
export function relinkFile(id: MediaId, path: string, allowMismatch: boolean): Promise<void> {
  return runJob("file", () => run(commands.relinkMedia(id, path, allowMismatch)), path);
}

/** Asks for the file of `id` (starting in its old folder), then relinks it. */
export async function locateFile(id: MediaId): Promise<void> {
  const item = useProjectStore.getState().state?.project.media.find((m) => m.id === id);
  if (!item) return;
  const path = await openDialog({
    multiple: false,
    directory: false,
    title: t.mediaBin.relink.locateTitle(item.fileName),
    defaultPath: parentDirOf(item.path),
    filters: [{ name: t.mediaBin.filterLabel, extensions: [...MEDIA_EXTENSIONS] }],
  });
  if (typeof path === "string") await relinkFile(id, path, false);
}

/** Keeps a `changed` file as it is now: relinks the item to its own path, accepting new content. */
export function keepAsIs(id: MediaId): Promise<void> {
  const item = useProjectStore.getState().state?.project.media.find((m) => m.id === id);
  return item ? relinkFile(id, item.path, true) : Promise.resolve();
}

/** The user confirmed the content mismatch. */
export function acceptMismatch(): Promise<void> {
  const { mismatch } = useRelinkStore.getState();
  return mismatch ? relinkFile(mismatch.id, mismatch.path, true) : Promise.resolve();
}

export function rejectMismatch(): void {
  set({ mismatch: null, result: null });
}

/** Asks for a folder and relinks every missing item found in it (one undo step). */
export async function searchFolder(): Promise<void> {
  const folder = await openDialog({ directory: true, multiple: false, title: t.mediaBin.relink.folderTitle });
  if (typeof folder === "string") await runJob("folder", () => run(commands.relinkMediaFolder(folder)));
}

export function cancelRelink(): void {
  const jobId = useRelinkStore.getState().running?.jobId;
  if (jobId) void run(commands.cancelJob(jobId));
}
