import type { IpcError, MediaErrorKind, MediaId, MediaStatus, MediaView_Serialize } from "@/bindings";
import type { JobOutcome } from "@/ipc/jobResults";

/** Media whose file is missing (`offline`) or has other content (`changed`) needs a relink. */
export function needsRelink(item: Pick<MediaView_Serialize, "status">): boolean {
  return item.status !== "online";
}

export type MissingCounts = { offline: number; changed: number };

export function countMissing(media: readonly Pick<MediaView_Serialize, "status">[]): MissingCounts {
  const counts: MissingCounts = { offline: 0, changed: 0 };
  for (const { status } of media) {
    if (status === "offline") counts.offline += 1;
    else if (status === "changed") counts.changed += 1;
  }
  return counts;
}

/** What the relink dialog shows after a relink job. */
export type RelinkResult = {
  relinked: MediaId[];
  notFound: MediaId[];
  /** `relinkMedia` found other content: ask, then relink again with `allowMismatch`. */
  mismatch: MediaId | null;
  failed: { path: string; error: MediaErrorKind }[];
  /** The whole job failed (folder unreadable); `null` when it ran, also when cancelled. */
  jobError: IpcError["kind"] | null;
  cancelled: boolean;
};

export function summarizeRelink({ files, error }: JobOutcome): RelinkResult {
  const result: RelinkResult = {
    relinked: [],
    notFound: [],
    mismatch: null,
    failed: [],
    jobError: null,
    cancelled: error?.kind === "cancelled",
  };
  if (error && !result.cancelled) result.jobError = error.kind;
  for (const { path, outcome } of files) {
    switch (outcome.kind) {
      case "relinked":
        result.relinked.push(outcome.media);
        break;
      case "notFound":
        result.notFound.push(outcome.media);
        break;
      case "fingerprintMismatch":
        result.mismatch = outcome.media;
        break;
      case "failed":
        result.failed.push({ path, error: outcome.error });
        break;
      case "added":
      case "alreadyImported":
        break;
    }
  }
  return result;
}

/**
 * React key of an item's poster or waveform: its URL plus its status. A relink to the same path
 * changes only the status (not `previewVersion`), so a preview that failed while the file was
 * offline (`503`) is asked again when the item comes back online.
 */
export function previewKey(url: string, status: MediaStatus): string {
  return `${url}#${status}`;
}

/** The folder part of a Windows or POSIX path, for the file dialog's start folder. */
export function parentDirOf(path: string): string | undefined {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut > 0 ? path.slice(0, cut) : undefined;
}
