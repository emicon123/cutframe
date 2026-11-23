import { describe, expect, it } from "vitest";

import type { IpcError } from "@/bindings";
import { t } from "@/i18n";

import { createRecent, isMissing, type RecentDeps, type RecentProject } from "./recent";

const trip: RecentProject = {
  path: "C:\\v\\Trip.cutframe",
  name: "Trip",
  duration: 0,
  openedAtMs: 2_000,
  modifiedMs: 1_000,
  cover: null,
};
const gone: RecentProject = { ...trip, path: "D:\\gone.cutframe", name: "Gone", modifiedMs: null };

const invalidEdit: IpcError = { kind: "invalidEdit", message: "exists" };
const io: IpcError = { kind: "io", message: "denied" };

type Fake = RecentDeps & { calls: string[] };

function fake(options: { failing?: Partial<Record<string, IpcError>> } = {}): Fake {
  const calls: string[] = [];
  const step =
    (name: string) =>
    (...args: string[]) => {
      calls.push([name, ...args].join(" "));
      const error = options.failing?.[name];
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      return error ? Promise.reject(error) : Promise.resolve(null);
    };
  return {
    calls,
    list: () => {
      calls.push("list");
      return Promise.resolve([trip, gone]);
    },
    remove: step("remove"),
    rename: step("rename"),
    duplicate: step("duplicate"),
    trash: step("trash"),
    reveal: step("reveal"),
    openPath: async (path) => {
      calls.push(`openPath ${path}`);
      await Promise.resolve();
    },
    reportError: (error) => calls.push(`error ${(error as IpcError).kind}`),
    clearError: () => undefined,
    logError: () => undefined,
  };
}

describe("recent list", () => {
  it("marks a file without a modification time as missing", () => {
    expect(isMissing(trip)).toBe(false);
    expect(isMissing(gone)).toBe(true);
  });

  it("load keeps missing entries (listing never prunes)", async () => {
    const recent = createRecent(fake());
    await recent.load();
    expect(recent.store.getState()).toEqual({ projects: [trip, gone], loaded: true });
  });

  it.each([
    ["remove", (r: ReturnType<typeof createRecent>) => r.remove(gone.path), `remove ${gone.path}`],
    ["trash", (r: ReturnType<typeof createRecent>) => r.trash(trip.path), `trash ${trip.path}`],
    ["reveal", (r: ReturnType<typeof createRecent>) => r.reveal(trip.path), `reveal ${trip.path}`],
    ["open", (r: ReturnType<typeof createRecent>) => r.open(trip.path), `openPath ${trip.path}`],
  ])("%s relists afterwards", async (_name, action, call) => {
    const deps = fake();
    await action(createRecent(deps));
    expect(deps.calls).toEqual([call, "list"]);
  });

  it("duplicate passes the translated copy name", async () => {
    const deps = fake();
    await createRecent(deps).duplicate(trip);
    expect(deps.calls).toEqual([`duplicate ${trip.path} ${t.home.copyName("Trip")}`, "list"]);
  });

  it("a failed action reports the error and still relists", async () => {
    const deps = fake({ failing: { trash: io } });
    await createRecent(deps).trash(trip.path);
    expect(deps.calls).toEqual([`trash ${trip.path}`, "error io", "list"]);
  });

  it("rename trims the name and resolves null on success", async () => {
    const deps = fake();
    await expect(createRecent(deps).rename(trip.path, "  Holiday ")).resolves.toBeNull();
    expect(deps.calls).toEqual([`rename ${trip.path} Holiday`, "list"]);
  });

  it("rename translates invalidEdit to the name conflict text, other kinds as usual", async () => {
    const conflict = createRecent(fake({ failing: { rename: invalidEdit } }));
    await expect(conflict.rename(trip.path, "Other")).resolves.toBe(t.home.rename.conflict);
    const failed = createRecent(fake({ failing: { rename: io } }));
    await expect(failed.rename(trip.path, "Other")).resolves.toBe(t.errors.kinds.io);
  });
});
