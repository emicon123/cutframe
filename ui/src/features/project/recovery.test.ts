import { describe, expect, it } from "vitest";

import type { IpcError } from "@/bindings";
import { t } from "@/i18n";

import {
  createRecovery,
  formatSavedAt,
  type RecoveryDeps,
  type RecoveryEntry,
  sortEntries,
} from "./recovery";

const older: RecoveryEntry = {
  id: "a",
  projectName: "Trip",
  filePath: "C:\\v\\trip.cutframe",
  savedAtMs: 1_000,
};
const newer: RecoveryEntry = { id: "b", projectName: "Draft", filePath: null, savedAtMs: 2_000 };

const notFound: IpcError = { kind: "fileNotFound", message: "gone" };

type Fake = RecoveryDeps & { calls: string[] };

function fake(options: {
  entries?: RecoveryEntry[];
  /** What the unsaved prompt decides: `true` lets the action run. */
  proceed?: boolean;
  failing?: string;
}): Fake {
  const calls: string[] = [];
  const step = (name: string) => (id: string) => {
    calls.push(`${name} ${id}`);
    // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
    // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
    return options.failing === name ? Promise.reject(notFound) : Promise.resolve(null);
  };
  return {
    calls,
    listRecovery: () => {
      calls.push("listRecovery");
      return options.failing === "listRecovery"
        ? Promise.reject(new Error("no dir"))
        : Promise.resolve(options.entries ?? []);
    },
    restoreRecovery: step("restoreRecovery"),
    discardRecovery: step("discardRecovery"),
    replaceGuarded: async (action) => {
      calls.push("guard");
      if (options.proceed === false) return false;
      await action();
      return true;
    },
    logError: (error) => calls.push(`log ${String(error)}`),
  };
}

describe("recovery", () => {
  it("shows nothing for an empty list and loads only once", async () => {
    const deps = fake({ entries: [] });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.load();
    expect(recovery.store.getState().entries).toEqual([]);
    expect(deps.calls).toEqual(["listRecovery"]);
  });

  it("logs a failed list and shows nothing", async () => {
    const deps = fake({ failing: "listRecovery" });
    const recovery = createRecovery(deps);
    await recovery.load();
    expect(recovery.store.getState().entries).toEqual([]);
    expect(deps.calls).toHaveLength(2);
    expect(deps.calls[1]).toMatch(/^log /);
  });

  it("sorts entries newest first", async () => {
    expect(sortEntries([older, newer]).map((e) => e.id)).toEqual(["b", "a"]);
    const recovery = createRecovery(fake({ entries: [older, newer] }));
    await recovery.load();
    expect(recovery.store.getState().entries.map((e) => e.id)).toEqual(["b", "a"]);
  });

  it("Restore goes through the unsaved guard, then closes the dialog", async () => {
    const deps = fake({ entries: [older, newer] });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.restore("a");
    expect(deps.calls).toEqual(["listRecovery", "guard", "restoreRecovery a"]);
    expect(recovery.store.getState()).toMatchObject({ entries: [], error: null, busy: false });
  });

  it("Restore cancelled at the unsaved prompt keeps the dialog", async () => {
    const deps = fake({ entries: [older], proceed: false });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.restore("a");
    expect(deps.calls).toEqual(["listRecovery", "guard"]);
    expect(recovery.store.getState().entries).toEqual([older]);
  });

  it("a failed restore keeps the row and shows the translated kind", async () => {
    const deps = fake({ entries: [older], failing: "restoreRecovery" });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.restore("a");
    expect(recovery.store.getState()).toMatchObject({
      entries: [older],
      error: t.errors.kinds.fileNotFound,
      busy: false,
    });
  });

  it("Discard removes the row; the last one closes the dialog", async () => {
    const deps = fake({ entries: [older, newer] });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.discard("b");
    expect(recovery.store.getState().entries).toEqual([older]);
    await recovery.discard("a");
    expect(recovery.store.getState().entries).toEqual([]);
    expect(deps.calls).toEqual(["listRecovery", "discardRecovery b", "discardRecovery a"]);
  });

  it("a failed discard keeps the row", async () => {
    const deps = fake({ entries: [older], failing: "discardRecovery" });
    const recovery = createRecovery(deps);
    await recovery.load();
    await recovery.discard("a");
    expect(recovery.store.getState()).toMatchObject({ entries: [older], error: t.errors.kinds.fileNotFound });
  });

  it("Later closes without any command", async () => {
    const deps = fake({ entries: [older] });
    const recovery = createRecovery(deps);
    await recovery.load();
    recovery.later();
    expect(recovery.store.getState().entries).toEqual([]);
    expect(deps.calls).toEqual(["listRecovery"]);
  });

  it("formats the time in the user's locale", () => {
    const expected = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(
      new Date(1_700_000_000_000),
    );
    expect(formatSavedAt(1_700_000_000_000)).toBe(expected);
  });
});
