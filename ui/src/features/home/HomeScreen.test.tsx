import { mockConvertFileSrc } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { IpcError } from "@/bindings";
import { Toaster } from "@/features/notifications/Toaster";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { useAppStore } from "@/stores/appStore";

import { HomeScreen } from "./HomeScreen";
import { createRecent, formatModified, type RecentProject } from "./recent";

const trip: RecentProject = {
  path: "C:\\v\\Trip.cutframe",
  name: "Trip",
  duration: 125 * FLICKS_PER_SECOND,
  openedAtMs: 3_000,
  modifiedMs: Date.UTC(2026, 10, 20, 12),
  cover: "0123456789abcdef0123456789abcdef",
};
const long: RecentProject = {
  ...trip,
  path: "C:\\v\\Long.cutframe",
  name: "Long",
  duration: 3723 * FLICKS_PER_SECOND,
  cover: null,
};
const gone: RecentProject = { ...trip, path: "D:\\Gone.cutframe", name: "Gone", modifiedMs: null };

afterEach(() => {
  vi.restoreAllMocks();
});

/** The home screen over a fake backend whose list `projects` the actions may change. */
function setup(initial: RecentProject[], failRename?: IpcError) {
  mockConvertFileSrc("windows");
  let projects = initial;
  const calls: string[] = [];
  const step = (name: string) => (path: string, arg?: string) => {
    calls.push([name, path, arg].filter((part) => part !== undefined).join(" "));
    return Promise.resolve(null);
  };
  const recent = createRecent({
    list: () => {
      calls.push("list");
      return Promise.resolve(projects);
    },
    remove: (path) => {
      calls.push(`remove ${path}`);
      projects = projects.filter((p) => p.path !== path);
      return Promise.resolve(null);
    },
    rename: (path, name) => {
      calls.push(`rename ${path} ${name}`);
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      return failRename ? Promise.reject(failRename) : Promise.resolve(null);
    },
    duplicate: step("duplicate"),
    trash: (path) => {
      calls.push(`trash ${path}`);
      projects = projects.filter((p) => p.path !== path);
      return Promise.resolve(null);
    },
    reveal: step("reveal"),
    openPath: (path) => {
      calls.push(`openPath ${path}`);
      return Promise.resolve();
    },
    reportError: (error) => useAppStore.getState().reportError(error),
    logError: () => undefined,
  });
  render(
    <>
      <HomeScreen recent={recent} />
      <Toaster />
    </>,
  );
  return { calls };
}

const card = async (name: string) => {
  const button = await screen.findByRole("button", { name: t.home.openItem(name) });
  const item = button.closest("li");
  if (!item) throw new Error("no card");
  return item;
};

/** Opens a card's `⋯` menu with the keyboard (Radix ignores synthetic clicks without pointer data). */
async function openCardMenu(name: string) {
  fireEvent.keyDown(screen.getByRole("button", { name: t.home.cardMenu(name) }), { key: "Enter" });
  return screen.findByRole("menu");
}

describe("HomeScreen", () => {
  it("shows each card's name, duration and modified date, and the cover from media://", async () => {
    setup([trip, long]);
    const tripCard = await card("Trip");
    expect(tripCard.textContent).toContain("2:05");
    expect(tripCard.textContent).toContain(t.home.modified(formatModified(trip.modifiedMs ?? 0)));
    expect(tripCard.querySelector("img")?.getAttribute("src")).toContain(`cover/${trip.cover}`);

    const longCard = await card("Long");
    expect(longCard.textContent).toContain("1:02:03");
    expect(longCard.querySelector("img")).toBeNull();
    expect(longCard.textContent).toContain(t.home.noCover);
  });

  it("falls back to the placeholder when the cover fails to load (404)", async () => {
    setup([trip]);
    const img = (await card("Trip")).querySelector("img");
    if (!img) throw new Error("no cover");
    fireEvent.error(img);
    await waitFor(() => expect(screen.getByText(t.home.noCover)).toBeTruthy());
  });

  it("opens a project with one click", async () => {
    const { calls } = setup([trip]);
    fireEvent.click(await screen.findByRole("button", { name: t.home.openItem("Trip") }));
    await waitFor(() => expect(calls).toContain(`openPath ${trip.path}`));
    await waitFor(() => expect(calls.at(-1)).toBe("list"));
  });

  it("shows a missing file as Missing, not openable, with only Remove from list", async () => {
    const { calls } = setup([gone]);
    const goneCard = await card("Gone");
    expect(goneCard.textContent).toContain(t.home.missing);
    const open = within(goneCard).getByRole("button", { name: t.home.openItem("Gone") });
    expect((open as HTMLButtonElement).disabled).toBe(true);

    const menu = await openCardMenu("Gone");
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([t.home.actions.remove]);
    fireEvent.click(items[0] as HTMLElement);
    await waitFor(() => expect(screen.getByText(t.home.empty)).toBeTruthy());
    expect(calls).toEqual(["list", `remove ${gone.path}`, "list"]);
  });

  it("offers every action for an existing file", async () => {
    setup([trip]);
    await card("Trip");
    const menu = await openCardMenu("Trip");
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((item) => item.textContent),
    ).toEqual([
      t.home.actions.open,
      t.home.actions.rename,
      t.home.actions.duplicate,
      t.home.actions.reveal,
      t.home.actions.delete,
      t.home.actions.remove,
    ]);
  });

  it("Delete asks first; Cancel does nothing, confirming moves it to the Recycle Bin", async () => {
    const { calls } = setup([trip]);
    await card("Trip");

    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.delete }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain(t.home.delete.title("Trip"));
    fireEvent.click(within(dialog).getAllByRole("button", { name: t.home.delete.cancel })[0] as HTMLElement);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["list"]);

    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.delete }),
    );
    fireEvent.click(await screen.findByRole("button", { name: t.home.delete.confirm }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["list", `trash ${trip.path}`, "list"]);
    expect(screen.getByText(t.home.empty)).toBeTruthy();
  });

  it("Escape closes the delete confirmation without deleting", async () => {
    const { calls } = setup([trip]);
    await card("Trip");
    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.delete }),
    );
    fireEvent.keyDown(await screen.findByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["list"]);
  });

  it("Rename is prefilled, refuses an empty name and applies on Enter", async () => {
    const { calls } = setup([trip]);
    await card("Trip");
    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.rename }),
    );
    const input = await screen.findByRole<HTMLInputElement>("textbox", { name: t.home.rename.label });
    expect(input.value).toBe("Trip");

    fireEvent.change(input, { target: { value: "   " } });
    const apply = screen.getByRole<HTMLButtonElement>("button", { name: t.home.rename.apply });
    expect(apply.disabled).toBe(true);
    fireEvent.submit(input);
    expect(calls).toEqual(["list"]);

    fireEvent.change(input, { target: { value: "Holiday" } });
    fireEvent.submit(input);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual(["list", `rename ${trip.path} Holiday`, "list"]);
  });

  it("a name conflict keeps the rename dialog open with the explanation", async () => {
    setup([trip], { kind: "invalidEdit", message: "exists" });
    await card("Trip");
    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.rename }),
    );
    const input = await screen.findByRole("textbox", { name: t.home.rename.label });
    fireEvent.change(input, { target: { value: "Taken" } });
    fireEvent.submit(input);
    expect((await screen.findByRole("alert")).textContent).toBe(t.home.rename.conflict);
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("Duplicate passes the copy name and relists", async () => {
    const { calls } = setup([trip]);
    await card("Trip");
    fireEvent.click(
      within(await openCardMenu("Trip")).getByRole("menuitem", { name: t.home.actions.duplicate }),
    );
    await waitFor(() =>
      expect(calls).toEqual(["list", `duplicate ${trip.path} ${t.home.copyName("Trip")}`, "list"]),
    );
  });

  it("shows the empty state and a failed command's translated error", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    setup([]);
    expect(await screen.findByText(t.home.empty)).toBeTruthy();
    useAppStore.getState().reportError({ kind: "fileNotFound", message: "gone" } satisfies IpcError);
    const toast = await screen.findByRole("alert");
    expect(toast.textContent).toContain(t.errors.kinds.fileNotFound);
    expect(toast.textContent).not.toContain("gone");
  });
});
