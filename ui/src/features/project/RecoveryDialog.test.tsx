import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { commands, type IpcError, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

import { createFileActions } from "./fileActions";
import { createRecovery, formatSavedAt, type RecoveryEntry } from "./recovery";
import { RecoveryDialog } from "./RecoveryDialog";

const saved: RecoveryEntry = {
  id: "a",
  projectName: "Trip",
  filePath: "C:\\v\\trip.cutframe",
  savedAtMs: 1_000,
};
const unsaved: RecoveryEntry = { id: "b", projectName: "Draft", filePath: null, savedAtMs: 2_000 };

const malformed: IpcError = { kind: "malformedProject", message: "bad json" };

afterEach(() => {
  useAppStore.setState({ previewOccluders: 0 });
});

/** The real commands against the IPC mock, behind a guard for a clean project (no prompt). */
function setup(entries: RecoveryEntry[]) {
  const guard = createFileActions({
    newProject: () => Promise.resolve(),
    openProject: () => Promise.resolve(),
    saveProject: () => Promise.resolve(true),
    saveProjectAs: () => Promise.resolve(),
    confirmClose: () => Promise.resolve(),
    current: () => ({ name: "Untitled", filePath: null, dirty: false }),
    askUnsaved: () => Promise.resolve("cancel"),
    pickOpenPath: () => Promise.resolve(null),
    pickSavePath: () => Promise.resolve(null),
    reportError: () => undefined,
    showFailure: () => undefined,
    showView: () => undefined,
  });
  const calls: string[] = [];
  let failRestore = false;
  mockCommands({
    listRecovery: () => entries,
    restoreRecovery: ({ id }) => {
      calls.push(`restore ${id}`);
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      return failRestore ? Promise.reject(malformed) : null;
    },
    discardRecovery: ({ id }) => {
      calls.push(`discard ${id}`);
      return null;
    },
  });
  const recovery = createRecovery({
    listRecovery: () => commands.listRecovery(),
    restoreRecovery: (id) => commands.restoreRecovery(id),
    discardRecovery: (id) => commands.discardRecovery(id),
    replaceGuarded: (action) => guard.replaceGuarded(action),
    logError: () => undefined,
    showEditor: () => calls.push("showEditor"),
  });
  render(<RecoveryDialog recovery={recovery} />);
  return { recovery, calls, failNextRestore: () => (failRestore = true) };
}

describe("RecoveryDialog", () => {
  it("lists entries newest first with path or 'Not saved yet' and the time, hiding the preview", async () => {
    const { recovery } = setup([saved, unsaved]);
    await recovery.load();

    const items = await screen.findAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringContaining("Draft"),
      expect.stringContaining("Trip"),
    ]);
    expect(items[0]?.textContent).toContain(t.recovery.notSavedYet);
    expect(items[0]?.textContent).toContain(t.recovery.savedAt(formatSavedAt(2_000)));
    expect(items[1]?.textContent).toContain("C:\\v\\trip.cutframe");
    expect(useAppStore.getState().previewOccluders).toBe(1);
  });

  it("shows nothing for an empty list", async () => {
    const { recovery } = setup([]);
    await recovery.load();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().previewOccluders).toBe(0);
  });

  it("Restore calls restoreRecovery and closes", async () => {
    const { recovery, calls } = setup([saved, unsaved]);
    await recovery.load();

    fireEvent.click(await screen.findByRole("button", { name: t.recovery.restoreItem("Trip") }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["restore a", "showEditor"]);
    expect(useAppStore.getState().previewOccluders).toBe(0);
  });

  it("a failed restore shows the translated error inside the dialog and keeps the row", async () => {
    const { recovery, failNextRestore } = setup([saved]);
    failNextRestore();
    await recovery.load();

    fireEvent.click(await screen.findByRole("button", { name: t.recovery.restoreItem("Trip") }));
    expect((await screen.findByRole("alert")).textContent).toBe(t.errors.kinds.malformedProject);
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("Discard removes rows and closes on the last one", async () => {
    const { recovery, calls } = setup([saved, unsaved]);
    await recovery.load();

    fireEvent.click(await screen.findByRole("button", { name: t.recovery.discardItem("Draft") }));
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: t.recovery.discardItem("Trip") }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["discard b", "discard a"]);
  });

  it("Escape means Later: closes and keeps the files", async () => {
    const { recovery, calls } = setup([saved]);
    await recovery.load();

    fireEvent.keyDown(await screen.findByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual([]);
  });

  it("the Later button closes too", async () => {
    const { recovery, calls } = setup([saved]);
    await recovery.load();

    // Both the footer button and the ✕ are named "Later"; the footer one comes first.
    const [footer] = await screen.findAllByRole("button", { name: t.recovery.later });
    if (!footer) throw new Error("no Later button");
    fireEvent.click(footer);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual([]);
  });
});
