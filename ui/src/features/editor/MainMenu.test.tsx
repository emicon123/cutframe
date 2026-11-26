import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { MainMenu } from "./MainMenu";
import { ShortcutsDialog } from "./ShortcutsDialog";

const state: ProjectState = {
  revision: 1,
  project: {
    id: "p",
    name: "Trip",
    settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
    duration: 0,
    background: { kind: "color", color: "#000000ff" },
    mainTrack: "main",
    magnetic: true,
    tracks: [],
    media: [],
  },
  history: {
    canUndo: true,
    canRedo: false,
    undoLabel: "addTrack",
    redoLabel: null,
    entries: [{ label: "addTrack" }],
    current: 1,
  },
  filePath: null,
  dirty: false,
};

afterEach(() => {
  useProjectStore.setState({ state: null });
  useUiStore.setState({ historyPanelOpen: false, shortcutsOpen: false, projectSettingsOpen: false });
  useAppStore.setState({ previewOccluders: 0 });
});

/** Opens a top-level menu with the keyboard (Radix ignores synthetic clicks without pointer data). */
async function openMenu(label: string) {
  fireEvent.keyDown(screen.getByRole("button", { name: label }), { key: "Enter" });
  return screen.findByRole("menu");
}

describe("MainMenu", () => {
  it("shows File, Edit, View and Help", () => {
    render(<MainMenu />);
    for (const label of [t.menu.file, t.menu.edit, t.menu.view, t.menu.help]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
  });

  it("names the undo entry, shows its shortcut and disables redo", async () => {
    useProjectStore.setState({ state });
    render(<MainMenu />);
    const menu = await openMenu(t.menu.edit);
    expect(useAppStore.getState().previewOccluders).toBe(1);

    const undo = within(menu).getByRole("menuitem", { name: /Undo Add Track/ });
    expect(undo.textContent).toContain("Ctrl+Z");
    expect(undo.hasAttribute("data-disabled")).toBe(false);
    const redo = within(menu).getByRole("menuitem", { name: /Redo/ });
    expect(redo.hasAttribute("data-disabled")).toBe(true);
  });

  it("runs undo from the menu", async () => {
    let undos = 0;
    mockCommands({ undo: () => (undos++, true) });
    useProjectStore.setState({ state });
    render(<MainMenu />);
    const menu = await openMenu(t.menu.edit);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Undo Add Track/ }));
    await waitFor(() => expect(undos).toBe(1));
    await waitFor(() => expect(useAppStore.getState().previewOccluders).toBe(0));
  });

  it("disables Save before a project is loaded", async () => {
    render(<MainMenu />);
    const menu = await openMenu(t.menu.file);
    const save = within(menu).getByRole("menuitem", { name: new RegExp(`^${t.file.save}Ctrl`) });
    expect(save.hasAttribute("data-disabled")).toBe(true);
    expect(within(menu).getByRole("menuitem", { name: t.file.exit })).toBeTruthy();
  });

  it("toggles the history panel from View", async () => {
    render(<MainMenu />);
    const menu = await openMenu(t.menu.view);
    const toggle = within(menu).getByRole("menuitemcheckbox", { name: t.view.historyPanel });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(useUiStore.getState().historyPanelOpen).toBe(true);
  });

  it("opens the shortcut reference from Help", async () => {
    render(
      <>
        <MainMenu />
        <ShortcutsDialog />
      </>,
    );
    const menu = await openMenu(t.menu.help);
    fireEvent.click(within(menu).getByRole("menuitem", { name: new RegExp(t.view.keyboardShortcuts) }));
    expect(await screen.findByRole("dialog", { name: t.shortcutsDialog.title })).toBeTruthy();
  });
});

describe("Help menu", () => {
  it("opens the logs folder and the About dialog", async () => {
    const calls: string[] = [];
    mockCommands({ openLogsFolder: () => (calls.push("logs"), null) });
    render(<MainMenu />);
    let menu = await openMenu(t.menu.help);
    fireEvent.click(within(menu).getByRole("menuitem", { name: t.help.openLogs }));
    await waitFor(() => expect(calls).toEqual(["logs"]));

    menu = await openMenu(t.menu.help);
    fireEvent.click(within(menu).getByRole("menuitem", { name: t.help.about }));
    expect(useUiStore.getState().aboutOpen).toBe(true);
    useUiStore.setState({ aboutOpen: false });
  });
});

describe("ShortcutsDialog", () => {
  it("lists the registered shortcuts by menu and hides the preview", async () => {
    useUiStore.setState({ shortcutsOpen: true });
    render(<ShortcutsDialog />);
    const dialog = await screen.findByRole("dialog");
    expect(useAppStore.getState().previewOccluders).toBe(1);

    const edit = within(dialog).getByRole("region", { name: t.menu.edit });
    expect(within(edit).getByText(t.edit.redo)).toBeTruthy();
    expect(within(edit).getByText("Ctrl+Shift+Z")).toBeTruthy();
    expect(within(edit).getByText("Ctrl+Y")).toBeTruthy();
    const file = within(dialog).getByRole("region", { name: t.menu.file });
    expect(within(file).getByText("Ctrl+Shift+S")).toBeTruthy();
    // Actions without a shortcut are not listed.
    expect(within(dialog).queryByText(t.file.exit)).toBeNull();
    expect(within(dialog).queryByRole("region", { name: t.menu.help })).toBeNull();
  });
});
