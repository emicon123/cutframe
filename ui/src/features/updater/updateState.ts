import type { IpcError, UpdateInfo_Serialize, UpdateProgress_Serialize } from "@/bindings";

/** Where the update offered by the last check stands (F0.12). */
export type UpdateStatus =
  /** No newer release is known. */
  | { phase: "none" }
  /** A newer release was found; nothing is downloaded yet. */
  | { phase: "available"; info: UpdateInfo_Serialize }
  /** Byte counts are validated decimal strings (64-bit, ADR-0003 §3); `total` is `null` when unknown. */
  | { phase: "downloading"; info: UpdateInfo_Serialize; downloaded: string; total: string | null }
  /** Downloaded and verified: `restartApp` installs it. */
  | { phase: "ready"; info: UpdateInfo_Serialize }
  /** The download or the verification failed; installing again is allowed. */
  | { phase: "failed"; info: UpdateInfo_Serialize; error: IpcError };

export type UpdateState = {
  status: UpdateStatus;
  /** "Later": the prompt is not shown until a check, `ready` or `failed` brings it back. */
  hidden: boolean;
  /** A `checkForUpdate` call is running; a second one is not started. */
  checking: boolean;
};

export type UpdateEvent =
  | { type: "checkStarted" }
  | { type: "checkFinished"; info: UpdateInfo_Serialize | null }
  | { type: "checkFailed" }
  | { type: "hide" }
  /** The user pressed Update or Retry. */
  | { type: "installStarted" }
  /** `installUpdate` itself was rejected (no `update-progress` follows). */
  | { type: "installFailed"; error: IpcError }
  | { type: "progress"; event: UpdateProgress_Serialize };

export const INITIAL_UPDATE_STATE: UpdateState = {
  status: { phase: "none" },
  hidden: false,
  checking: false,
};

const DECIMAL = /^\d+$/;

/** A byte count from `update-progress`, or `null` when it is not a plain decimal string. */
export function parseByteCount(value: string | null | undefined): string | null {
  if (value === null || value === undefined) return null;
  const trimmed = value.trim();
  return DECIMAL.test(trimmed) ? BigInt(trimmed).toString() : null;
}

/**
 * Whole percent of the download (0–100, never rounded up to 100 before the end), or `null` when the
 * total is unknown. Computed with `BigInt`, so 64-bit counts stay exact.
 */
export function downloadPercent(downloaded: string, total: string | null): number | null {
  if (total === null) return null;
  const all = BigInt(total);
  if (all <= 0n) return null;
  const done = BigInt(downloaded);
  const percent = (done * 100n) / all;
  return Number(percent > 100n ? 100n : percent);
}

function hasInfo(status: UpdateStatus): status is Exclude<UpdateStatus, { phase: "none" }> {
  return status.phase !== "none";
}

/** The state after `event`. Pure, so every transition is unit-tested. */
export function reduceUpdate(state: UpdateState, event: UpdateEvent): UpdateState {
  const { status } = state;
  switch (event.type) {
    case "checkStarted":
      return { ...state, checking: true };
    case "checkFailed":
      return { ...state, checking: false };
    case "checkFinished": {
      const busy = status.phase === "downloading" || status.phase === "ready";
      if (event.info === null) {
        // Rust answers with the running download while there is one; never drop it.
        return { ...state, checking: false, status: busy ? status : { phase: "none" } };
      }
      // The same release that is downloading, ready or failed keeps its progress.
      const same =
        hasInfo(status) && status.phase !== "available" && status.info.version === event.info.version;
      return {
        checking: false,
        hidden: false,
        status: same ? { ...status, info: event.info } : { phase: "available", info: event.info },
      };
    }
    case "hide":
      return { ...state, hidden: true };
    case "installStarted":
      if (status.phase !== "available" && status.phase !== "failed") return state;
      return { ...state, status: { phase: "downloading", info: status.info, downloaded: "0", total: null } };
    case "installFailed":
      if (!hasInfo(status)) return state;
      return { ...state, hidden: false, status: { phase: "failed", info: status.info, error: event.error } };
    case "progress":
      return applyProgress(state, event.event);
  }
}

function applyProgress(state: UpdateState, progress: UpdateProgress_Serialize): UpdateState {
  const { status } = state;
  // An event without a known release (e.g. after a webview reload before the startup check
  // answered) is dropped; the next check brings the download back.
  if (!hasInfo(status)) return state;
  const { info } = status;
  switch (progress.state) {
    case "downloading": {
      // A late tick must not undo `ready`.
      if (status.phase === "ready") return state;
      const downloaded =
        parseByteCount(progress.downloaded) ?? (status.phase === "downloading" ? status.downloaded : "0");
      const parsedTotal = parseByteCount(progress.total);
      const total = parsedTotal === null || parsedTotal === "0" ? null : parsedTotal;
      return { ...state, status: { phase: "downloading", info, downloaded, total } };
    }
    case "ready":
      return { ...state, hidden: false, status: { phase: "ready", info } };
    case "failed":
      return { ...state, hidden: false, status: { phase: "failed", info, error: progress.error } };
  }
}
