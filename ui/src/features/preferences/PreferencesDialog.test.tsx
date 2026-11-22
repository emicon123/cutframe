import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type CacheInfo, mockCommands, type MockCommandHandlers } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { useAppStore } from "@/stores/appStore";
import { useUiStore } from "@/stores/uiStore";

import type { Preferences } from "./preferences";
import { PreferencesDialog } from "./PreferencesDialog";
import { usePreferencesStore } from "./preferencesStore";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const stored: Preferences = {
  schemaVersion: 1,
  autosaveInterval: 60 * FLICKS_PER_SECOND,
  defaultImageDuration: 5 * FLICKS_PER_SECOND,
  defaultTransitionDuration: FLICKS_PER_SECOND / 2,
  language: "en",
  cacheDir: null,
};

const cache = (sizeBytes: string, files: number): CacheInfo => ({
  path: "C:\\Users\\me\\AppData\\Local\\app.cutframe.editor\\cutframe-cache",
  sizeBytes,
  files,
});

function setup(handlers: Partial<MockCommandHandlers> = {}) {
  const sent: Preferences[] = [];
  mockCommands({
    getPreferences: () => stored,
    getCacheInfo: () => cache("1536", 2),
    setPreferences: ({ preferences }) => {
      const full = { ...stored, ...preferences } as Preferences;
      sent.push(full);
      return full;
    },
    ...handlers,
  });
  useUiStore.setState({ preferencesOpen: true });
  render(<PreferencesDialog />);
  return sent;
}

/** Radix tabs switch on mouse down, not on click. */
const openTab = async (name: string) => fireEvent.mouseDown(await screen.findByRole("tab", { name }));

beforeEach(() => {
  usePreferencesStore.setState({ preferences: stored, cacheInfo: null });
});

afterEach(() => {
  useUiStore.setState({ preferencesOpen: false, shortcutsOpen: false });
  useAppStore.setState({ previewOccluders: 0, info: null });
  vi.mocked(openDialog).mockReset();
  vi.restoreAllMocks();
});

describe("PreferencesDialog", () => {
  it("hides the preview while open", async () => {
    setup();
    await screen.findByRole("dialog");
    expect(useAppStore.getState().previewOccluders).toBe(1);
  });

  it("Save sends the full preferences in flicks and closes", async () => {
    const sent = setup();
    fireEvent.change(await screen.findByLabelText(t.preferences.general.autosaveInterval), {
      target: { value: "120" },
    });
    await openTab(t.preferences.tabs.editing);
    fireEvent.change(await screen.findByLabelText(t.preferences.editing.imageDuration), {
      target: { value: "2.5" },
    });
    fireEvent.click(screen.getByRole("button", { name: t.preferences.save }));

    await waitFor(() => expect(useUiStore.getState().preferencesOpen).toBe(false));
    expect(sent).toEqual([
      {
        ...stored,
        autosaveInterval: 120 * FLICKS_PER_SECOND,
        defaultImageDuration: 2.5 * FLICKS_PER_SECOND,
      },
    ]);
    expect(usePreferencesStore.getState().preferences?.autosaveInterval).toBe(120 * FLICKS_PER_SECOND);
  });

  it("sends null when autosave is switched off", async () => {
    const sent = setup();
    fireEvent.click(await screen.findByLabelText(t.preferences.general.autosave));
    expect(screen.getByText(t.preferences.general.autosaveOffHint)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: t.preferences.save }));
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]?.autosaveInterval).toBeNull();
  });

  it("keeps the dialog open and marks the field Rust rejected", async () => {
    const rejection = "defaultTransitionDuration is out of range";
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    setup({
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      setPreferences: () => Promise.reject({ kind: "invalidEdit", message: rejection }),
    });
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.save }));

    // The dialog switches to the tab of the rejected field.
    expect(await screen.findByText(t.preferences.fieldErrors.rejected)).toBeDefined();
    expect(screen.getByLabelText(t.preferences.editing.transitionDuration)).toHaveProperty(
      "ariaInvalid",
      "true",
    );
    expect(useUiStore.getState().preferencesOpen).toBe(true);
  });

  it("does not send an invalid value and shows the hint on its tab", async () => {
    const sent = setup();
    await openTab(t.preferences.tabs.editing);
    fireEvent.change(await screen.findByLabelText(t.preferences.editing.transitionDuration), {
      target: { value: "20" },
    });
    expect(screen.getByText(t.preferences.fieldErrors.outOfRange("0.1", "10"))).toBeDefined();
    await openTab(t.preferences.tabs.general);
    fireEvent.click(screen.getByRole("button", { name: t.preferences.save }));
    expect(await screen.findByLabelText(t.preferences.editing.transitionDuration)).toBeDefined();
    expect(sent).toEqual([]);
  });

  it("Cancel drops the draft", async () => {
    const sent = setup();
    fireEvent.change(await screen.findByLabelText(t.preferences.general.autosaveInterval), {
      target: { value: "300" },
    });
    fireEvent.click(screen.getByRole("button", { name: t.preferences.cancel }));
    await waitFor(() => expect(useUiStore.getState().preferencesOpen).toBe(false));
    expect(sent).toEqual([]);
  });

  it("loads the preferences when opened before startup loaded them", async () => {
    usePreferencesStore.setState({ preferences: null });
    setup();
    expect(await screen.findByLabelText(t.preferences.general.autosaveInterval)).toHaveProperty(
      "value",
      "60",
    );
  });

  it("opens the shortcut reference from the Shortcuts tab", async () => {
    setup();
    await openTab(t.preferences.tabs.shortcuts);
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.shortcuts.show }));
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
  });

  it("shows the GPU report on the Performance tab", async () => {
    useAppStore.setState({
      info: {
        version: "0.1.0",
        previewMode: "native",
        gpu: {
          adapter: "Microsoft Basic Render Driver",
          backend: "Dx12",
          software: true,
          deviceType: "software",
          driver: "",
          presentsToWindow: true,
          fallback: "noHardwareAdapter",
          videoAcceleration: null,
        },
      },
    });
    setup();
    await openTab(t.preferences.tabs.performance);
    expect(await screen.findByText("Microsoft Basic Render Driver")).toBeDefined();
    expect(screen.getByText(t.gpu.deviceTypes.software)).toBeDefined();
    expect(screen.getByText(t.gpu.fallbacks.noHardwareAdapter)).toBeDefined();
    expect(screen.getByText(t.preferences.performance.notReported)).toBeDefined();
    expect(screen.getByText(t.preferences.performance.videoAccelerationUnknown)).toBeDefined();
  });
});

describe("Cache tab", () => {
  it("shows the size and clears the cache after confirming", async () => {
    let cleared = 0;
    setup({
      clearCache: () => {
        cleared += 1;
        return cache("0", 0);
      },
    });
    await openTab(t.preferences.tabs.cache);
    expect(
      await screen.findByText(`${t.units.scaled("1.5", "KB")} · ${t.preferences.cache.files(2)}`),
    ).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: t.preferences.cache.clear }));
    expect(cleared).toBe(0);
    fireEvent.click(screen.getByRole("button", { name: t.preferences.cache.confirm }));
    expect(await screen.findByText(`${t.units.bytes("0")} · ${t.preferences.cache.files(0)}`)).toBeDefined();
    expect(cleared).toBe(1);
  });

  it("does nothing when the confirmation is cancelled", async () => {
    let cleared = 0;
    setup({
      clearCache: () => {
        cleared += 1;
        return cache("0", 0);
      },
    });
    await openTab(t.preferences.tabs.cache);
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.cache.clear }));
    fireEvent.click(screen.getByRole("button", { name: t.preferences.cache.cancel }));
    expect(screen.getByRole("button", { name: t.preferences.cache.clear })).toBeDefined();
    expect(cleared).toBe(0);
  });

  it("changes the folder at once, keeping the stored values (not the draft)", async () => {
    vi.mocked(openDialog).mockResolvedValue("D:\\Cache");
    const sent = setup();
    fireEvent.change(await screen.findByLabelText(t.preferences.general.autosaveInterval), {
      target: { value: "300" },
    });
    await openTab(t.preferences.tabs.cache);
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.cache.change }));

    await waitFor(() => expect(sent).toEqual([{ ...stored, cacheDir: "D:\\Cache" }]));
    expect(vi.mocked(openDialog)).toHaveBeenCalledWith(expect.objectContaining({ directory: true }));
    await waitFor(() => expect(usePreferencesStore.getState().preferences?.cacheDir).toBe("D:\\Cache"));
    expect(screen.getByRole("button", { name: t.preferences.cache.resetFolder })).toHaveProperty(
      "disabled",
      false,
    );
  });

  it("does nothing when the folder picker is cancelled", async () => {
    vi.mocked(openDialog).mockResolvedValue(null);
    const sent = setup();
    await openTab(t.preferences.tabs.cache);
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.cache.change }));
    await waitFor(() => expect(vi.mocked(openDialog)).toHaveBeenCalled());
    expect(sent).toEqual([]);
  });

  it("shows a failed clear in the tab", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    setup({
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- an `IpcError`
      clearCache: () => Promise.reject({ kind: "io", message: "access denied" }),
    });
    await openTab(t.preferences.tabs.cache);
    fireEvent.click(await screen.findByRole("button", { name: t.preferences.cache.clear }));
    fireEvent.click(screen.getByRole("button", { name: t.preferences.cache.confirm }));
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", t.errors.kinds.io);
  });
});
