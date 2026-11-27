import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { events, type IpcError, mockCommands, type UpdateInfo_Serialize } from "@/bindings";
import { answerUnsaved, useUnsavedPrompt } from "@/features/project/unsavedPrompt";
import { t } from "@/i18n";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useToastStore } from "@/stores/toastStore";

import { UpdatePrompt } from "./UpdatePrompt";
import {
  checkForUpdates,
  checkOnStartup,
  hideUpdatePrompt,
  installUpdate,
  restartToUpdate,
  startUpdateSync,
  useUpdateStore,
} from "./updater";
import {
  downloadPercent,
  INITIAL_UPDATE_STATE,
  parseByteCount,
  reduceUpdate,
  type UpdateEvent,
  type UpdateState,
} from "./updateState";

const info: UpdateInfo_Serialize = {
  version: "0.2.0",
  currentVersion: "0.1.0",
  date: null,
  notes: "Faster export",
};
const offline: IpcError = { kind: "unavailable", message: "update check disabled" };
const rejected: IpcError = { kind: "updateRejected", message: "bad signature" };

/** Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`. */
function rejectAsTauri(error: IpcError): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
  return Promise.reject(error);
}

function run(...steps: UpdateEvent[]): UpdateState {
  return steps.reduce(reduceUpdate, INITIAL_UPDATE_STATE);
}

const found: UpdateEvent = { type: "checkFinished", info };
const tick = (downloaded: string, total: string | null): UpdateEvent => ({
  type: "progress",
  event: { state: "downloading", downloaded, total },
});

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  vi.restoreAllMocks();
  useUpdateStore.setState({ ...INITIAL_UPDATE_STATE, startupChecked: false, restarting: false });
  useProjectStore.setState({ state: null });
});

describe("parseByteCount / downloadPercent", () => {
  it("accepts plain decimal strings only, 64-bit exact", () => {
    expect(parseByteCount("1024")).toBe("1024");
    expect(parseByteCount(" 007 ")).toBe("7");
    expect(parseByteCount("18446744073709551615")).toBe("18446744073709551615");
    for (const bad of ["", "-1", "1.5", "1e6", "abc", null, undefined])
      expect(parseByteCount(bad)).toBeNull();
  });

  it("gives whole percent, capped at 100, or null without a total", () => {
    expect(downloadPercent("0", "200")).toBe(0);
    expect(downloadPercent("199", "200")).toBe(99);
    expect(downloadPercent("200", "200")).toBe(100);
    expect(downloadPercent("300", "200")).toBe(100);
    expect(downloadPercent("50", null)).toBeNull();
    expect(downloadPercent("50", "0")).toBeNull();
    expect(downloadPercent("9007199254740993", "18014398509481986")).toBe(50);
  });
});

describe("reduceUpdate", () => {
  it("goes found → downloading → ready", () => {
    expect(run(found).status).toEqual({ phase: "available", info });
    expect(run(found, { type: "installStarted" }).status).toEqual({
      phase: "downloading",
      info,
      downloaded: "0",
      total: null,
    });
    expect(run(found, { type: "installStarted" }, tick("10", "100")).status).toMatchObject({
      downloaded: "10",
      total: "100",
    });
    const ready = run(found, { type: "installStarted" }, tick("100", "100"), {
      type: "progress",
      event: { state: "ready" },
    });
    expect(ready.status).toEqual({ phase: "ready", info });
    // A late tick does not undo ready.
    expect(reduceUpdate(ready, tick("50", "100")).status.phase).toBe("ready");
  });

  it("keeps the last count on a malformed tick and treats a zero total as unknown", () => {
    const state = run(found, { type: "installStarted" }, tick("40", "100"), tick("x", "0"));
    expect(state.status).toMatchObject({ phase: "downloading", downloaded: "40", total: null });
  });

  it("a failure shows the prompt again and Retry restarts the download", () => {
    const failed = run(
      found,
      { type: "hide" },
      { type: "installStarted" },
      {
        type: "progress",
        event: { state: "failed", error: rejected },
      },
    );
    expect(failed).toMatchObject({ hidden: false, status: { phase: "failed", error: rejected } });
    expect(reduceUpdate(failed, { type: "installStarted" }).status.phase).toBe("downloading");
    expect(run(found, { type: "installFailed", error: offline }).status).toMatchObject({ phase: "failed" });
  });

  it("drops progress without a known release", () => {
    expect(run(tick("1", "2"), { type: "progress", event: { state: "ready" } })).toEqual(
      INITIAL_UPDATE_STATE,
    );
    expect(run({ type: "installStarted" })).toEqual(INITIAL_UPDATE_STATE);
  });

  it("a new check keeps a running download of the same release and unhides the prompt", () => {
    const downloading = run(found, { type: "installStarted" }, tick("5", "10"), { type: "hide" });
    const again = reduceUpdate(downloading, found);
    expect(again.hidden).toBe(false);
    expect(again.status).toMatchObject({ phase: "downloading", downloaded: "5" });
    expect(reduceUpdate(downloading, { type: "checkFinished", info: null }).status.phase).toBe("downloading");
    const newer = { ...info, version: "0.3.0" };
    expect(
      reduceUpdate(run(found, { type: "installFailed", error: offline }), {
        type: "checkFinished",
        info: newer,
      }).status,
    ).toEqual({
      phase: "available",
      info: newer,
    });
  });

  it("tracks a running check", () => {
    expect(run({ type: "checkStarted" }).checking).toBe(true);
    expect(run({ type: "checkStarted" }, { type: "checkFailed" }).checking).toBe(false);
    expect(run({ type: "checkStarted" }, { type: "checkFinished", info: null })).toEqual(
      INITIAL_UPDATE_STATE,
    );
  });
});

describe("update checks", () => {
  it("the startup check ignores failures and runs once", async () => {
    const console = vi.spyOn(globalThis.console, "error").mockImplementation(() => undefined);
    let calls = 0;
    mockCommands({ checkForUpdate: () => (calls++, rejectAsTauri(offline)) });
    checkOnStartup();
    checkOnStartup();
    await vi.waitFor(() => expect(useUpdateStore.getState().checking).toBe(false));
    expect(calls).toBe(1);
    expect(useToastStore.getState().toasts).toEqual([]);
    expect(useUpdateStore.getState().status.phase).toBe("none");
    expect(console).toHaveBeenCalled();
  });

  it("the startup check shows a found update", async () => {
    mockCommands({ checkForUpdate: () => info });
    checkOnStartup();
    await vi.waitFor(() => expect(useUpdateStore.getState().status.phase).toBe("available"));
  });

  it("a manual check says up to date or shows the failure", async () => {
    mockCommands({ checkForUpdate: () => null });
    await checkForUpdates({ manual: true });
    expect(useToastStore.getState().toasts.map((toast) => toast.text)).toEqual([t.updater.upToDate]);

    vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockCommands({ checkForUpdate: () => rejectAsTauri(offline) });
    await checkForUpdates({ manual: true });
    expect(useToastStore.getState().toasts.at(-1)).toMatchObject({
      tone: "error",
      text: t.updater.checkFailed,
      detail: t.updater.unreachable,
    });
  });
});

describe("UpdatePrompt", () => {
  it("offers the update with notes, downloads with progress and restarts", async () => {
    const calls: string[] = [];
    mockCommands({
      checkForUpdate: () => info,
      installUpdate: () => (calls.push("install"), null),
      restartApp: () => (calls.push("restart"), null),
    });
    stop = startUpdateSync();
    render(<UpdatePrompt />);
    await checkForUpdates({ manual: false });

    expect(await screen.findByRole("heading", { name: t.updater.available("0.2.0") })).toBeTruthy();
    expect(screen.queryByText("Faster export")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: t.updater.showNotes }));
    expect(screen.getByText("Faster export")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: t.updater.update }));
    await vi.waitFor(() => expect(calls).toEqual(["install"]));
    await events.updateProgress.emit({ state: "downloading", downloaded: "1048576", total: "4194304" });
    const bar = await screen.findByRole("progressbar", { name: t.updater.progressLabel });
    expect(bar.getAttribute("aria-valuenow")).toBe("25");

    await events.updateProgress.emit({ state: "ready" });
    fireEvent.click(await screen.findByRole("button", { name: t.updater.restart }));
    await vi.waitFor(() => expect(calls).toEqual(["install", "restart"]));
  });

  it("shows bytes without a total, and Later hides it", async () => {
    mockCommands({ checkForUpdate: () => info, installUpdate: () => null });
    stop = startUpdateSync();
    render(<UpdatePrompt />);
    await checkForUpdates({ manual: false });
    await installUpdate();
    await events.updateProgress.emit({ state: "downloading", downloaded: "2048", total: null });
    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBeNull();
    expect(screen.getByText("2.0 KB")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t.updater.later }));
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows a failed download with its reason and Retry", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const calls: string[] = [];
    mockCommands({ checkForUpdate: () => info, installUpdate: () => (calls.push("install"), null) });
    stop = startUpdateSync();
    render(<UpdatePrompt />);
    await checkForUpdates({ manual: false });
    await installUpdate();
    await events.updateProgress.emit({ state: "failed", error: rejected });
    expect(await screen.findByText(t.errors.kinds.updateRejected)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t.updater.retry }));
    await vi.waitFor(() => expect(calls).toEqual(["install", "install"]));
  });

  it("asks about unsaved changes before restarting, and Cancel keeps the app", async () => {
    useProjectStore.setState({
      state: { project: { name: "Trip" }, filePath: null, dirty: true } as ProjectState,
    });
    const restart = vi.fn(() => null);
    mockCommands({ restartApp: restart });
    useUpdateStore.setState({ status: { phase: "ready", info } });
    const pending = restartToUpdate();
    await vi.waitFor(() => expect(useUnsavedPrompt.getState().resolve).not.toBeNull());
    answerUnsaved("cancel");
    await pending;
    expect(restart).not.toHaveBeenCalled();

    const again = restartToUpdate();
    await vi.waitFor(() => expect(useUnsavedPrompt.getState().resolve).not.toBeNull());
    answerUnsaved("discard");
    await again;
    expect(restart).toHaveBeenCalledOnce();
    hideUpdatePrompt();
  });
});
