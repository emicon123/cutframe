import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  type ExportCapabilities,
  type ExportCheck_Serialize,
  type ExportIssue,
  type ExportSettings_Deserialize,
  type IpcError,
  mockCommands,
  type MockCommandHandlers,
} from "@/bindings";
import { useRelinkStore } from "@/features/media-bin/relinkStore";
import { projectState, S } from "@/features/inspector/testProject";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";

import { ExportDialog } from "./ExportDialog";
import { initialExportState, useExportStore } from "./exportStore";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ videoDir: () => Promise.resolve("C:\\Users\\me\\Videos") }));

const noCaps: ExportCapabilities = { probed: false, encoders: [] };
const withHevc: ExportCapabilities = {
  probed: true,
  encoders: [
    { codec: "h264", encoder: "nvenc", hardware: true, available: true },
    { codec: "hevc", encoder: "nvenc", hardware: true, available: true },
  ],
};

function result(issues: ExportIssue[] = [], canStart = true): ExportCheck_Serialize {
  return {
    width: 1920,
    height: 1080,
    frameRate: { num: 30, den: 1 },
    duration: 10 * S,
    frames: 300,
    videoKbps: 12000,
    audioKbps: 384,
    sampleRate: 48000,
    estimatedBytes: "16128000",
    freeBytes: null,
    encoder: "nvenc",
    issues,
    canStart,
  };
}

type Setup = {
  checks: ExportSettings_Deserialize[];
  starts: { settings: ExportSettings_Deserialize; overwrite: boolean }[];
};

function setup(handlers: Partial<MockCommandHandlers> = {}, check = result()): Setup {
  const calls: Setup = { checks: [], starts: [] };
  mockCommands({
    getExportCapabilities: () => noCaps,
    checkExport: ({ settings }) => {
      calls.checks.push(settings);
      return check;
    },
    startExport: ({ settings, overwrite }) => {
      calls.starts.push({ settings, overwrite });
      return "job-1";
    },
    ...handlers,
  });
  useExportStore.setState({ dialogOpen: true });
  render(<ExportDialog />);
  return calls;
}

/** Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`. */
function rejectWith(kind: IpcError["kind"], message: string): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
  return Promise.reject({ kind, message });
}

const startButton = () => screen.getByRole("button", { name: t.export.start });
const lastPath = (calls: Setup) => calls.checks.at(-1)?.path;

beforeEach(() => {
  window.localStorage.clear();
  useProjectStore.setState({ state: projectState([]) });
});

afterEach(() => {
  useExportStore.setState(initialExportState);
  useAppStore.setState({ previewOccluders: 0 });
  useProjectStore.setState({ state: null });
  useRelinkStore.setState({ open: false, focus: null });
  vi.mocked(openDialog).mockReset();
});

describe("ExportDialog", () => {
  it("hides the preview while open and checks the default settings", async () => {
    const calls = setup();
    await screen.findByRole("dialog");
    expect(useAppStore.getState().previewOccluders).toBe(1);
    await screen.findByText(/1920 × 1080 · 30 fps · H\.264 12 Mbps · AAC 384 kbps/);
    expect(calls.checks.at(-1)).toEqual({
      path: "C:\\Users\\me\\Videos\\Trip.mp4",
      target: {
        kind: "video",
        settings: {
          codec: "h264",
          resolution: "p1080",
          frameRate: null,
          quality: { kind: "recommended" },
          encoder: null,
        },
      },
    });
    expect(screen.getByLabelText(t.export.preset)).toHaveProperty("value", "youtube1080");
    expect(startButton()).toHaveProperty("disabled", false);
  });

  it("starts in the remembered folder", async () => {
    window.localStorage.setItem("export.lastFolder.v1", "E:\\Exports");
    const calls = setup();
    await waitFor(() => expect(lastPath(calls)).toBe("E:\\Exports\\Trip.mp4"));
  });

  it("lists the issues with media names and keeps Export disabled while one blocks", async () => {
    setup({}, result([{ kind: "mediaOffline", media: "a" }, { kind: "lowDiskSpace" }], false));
    expect(await screen.findByText(t.export.issues.mediaOffline("a.file"))).toBeTruthy();
    expect(screen.getByText(t.export.issues.lowDiskSpace)).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);

    fireEvent.click(screen.getByRole("button", { name: t.export.relink }));
    expect(useExportStore.getState().dialogOpen).toBe(false);
    expect(useRelinkStore.getState()).toMatchObject({ open: true, focus: "a" });
  });

  it("starts the export, remembers the folder and closes", async () => {
    const calls = setup();
    await waitFor(() => expect(startButton()).toHaveProperty("disabled", false));
    fireEvent.click(startButton());
    await waitFor(() => expect(useExportStore.getState().dialogOpen).toBe(false));
    expect(calls.starts).toEqual([{ settings: calls.checks.at(-1), overwrite: false }]);
    expect(useExportStore.getState().job?.id).toBe("job-1");
    expect(window.localStorage.getItem("export.lastFolder.v1")).toBe("C:\\Users\\me\\Videos");
  });

  it("asks before replacing an existing file", async () => {
    const calls = setup({}, result([{ kind: "fileExists" }]));
    expect(await screen.findByText(t.export.issues.fileExists)).toBeTruthy();
    await waitFor(() => expect(startButton()).toHaveProperty("disabled", false));
    fireEvent.click(startButton());
    expect(screen.getByText(t.export.replaceQuestion("Trip.mp4"))).toBeTruthy();
    expect(calls.starts).toEqual([]);

    // Back returns to the form; Replace starts with overwrite.
    fireEvent.click(screen.getByRole("button", { name: t.export.back }));
    fireEvent.click(startButton());
    fireEvent.click(screen.getByRole("button", { name: t.export.replace }));
    await waitFor(() => expect(calls.starts).toEqual([{ settings: calls.checks.at(-1), overwrite: true }]));
  });

  it("asks when the file appeared after the check", async () => {
    let first = true;
    const calls = setup({
      startExport: ({ settings, overwrite }) => {
        calls.starts.push({ settings, overwrite });
        if (first) {
          first = false;
          return rejectWith("fileExists", "exists");
        }
        return "job-2";
      },
    });
    await waitFor(() => expect(startButton()).toHaveProperty("disabled", false));
    fireEvent.click(startButton());
    fireEvent.click(await screen.findByRole("button", { name: t.export.replace }));
    await waitFor(() => expect(useExportStore.getState().job?.id).toBe("job-2"));
    expect(calls.starts.map((s) => s.overwrite)).toEqual([false, true]);
  });

  it("shows a failed start in words", async () => {
    setup({
      startExport: () => rejectWith("busy", "an export runs"),
    });
    await waitFor(() => expect(startButton()).toHaveProperty("disabled", false));
    fireEvent.click(startButton());
    expect((await screen.findByRole("alert")).textContent).toBe(t.errors.kinds.busy);
    expect(useExportStore.getState().dialogOpen).toBe(true);
  });

  it("switches the preset label to Custom and checks the custom settings", async () => {
    const calls = setup();
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: t.export.custom }));
    fireEvent.change(screen.getByLabelText(t.export.quality), { target: { value: "custom" } });
    fireEvent.change(screen.getByLabelText(t.export.customBitrate), { target: { value: "20" } });
    fireEvent.change(screen.getByLabelText(t.export.frameRate), { target: { value: "60/1" } });
    expect(screen.getByLabelText(t.export.preset)).toHaveProperty("value", "custom");
    await waitFor(() =>
      expect(calls.checks.at(-1)?.target.settings).toMatchObject({
        frameRate: { num: 60, den: 1 },
        quality: { kind: "custom", kbps: 20000 },
      }),
    );

    // Picking a preset again resets the video fields.
    fireEvent.change(screen.getByLabelText(t.export.preset), { target: { value: "youtube4k" } });
    await waitFor(() =>
      expect(calls.checks.at(-1)?.target.settings).toMatchObject({ resolution: "p2160", frameRate: null }),
    );
  });

  it("offers HEVC only once the probe found a hardware HEVC encoder", async () => {
    setup();
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: t.export.custom }));
    const codec = screen.getByLabelText(t.export.codec);
    const hevc = within(codec).getByRole("option", { name: t.export.hevcUnavailable });
    expect(hevc).toHaveProperty("disabled", true);
    expect(screen.getByText(t.export.probing)).toBeTruthy();

    useExportStore.setState({ capabilities: withHevc });
    const enabled = await within(codec).findByRole("option", { name: t.export.codecs.hevc });
    expect(enabled).toHaveProperty("disabled", false);
    fireEvent.change(codec, { target: { value: "hevc" } });
    expect(screen.getByText(t.export.hevcHint)).toBeTruthy();
    const encoder = screen.getByLabelText(t.export.encoder);
    expect(within(encoder).getByRole("option", { name: t.export.encoders.nvenc })).toBeTruthy();
  });

  it("asks for a name and a folder before checking", async () => {
    const calls = setup();
    await waitFor(() => expect(calls.checks.length).toBeGreaterThan(0));
    fireEvent.change(screen.getByLabelText(t.export.fileName), { target: { value: " " } });
    expect(screen.getByText(t.export.formErrors.name)).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);
  });

  it("picks the folder with the native folder picker", async () => {
    vi.mocked(openDialog).mockResolvedValue("F:\\Out");
    const calls = setup();
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: t.export.browse }));
    await waitFor(() => expect(lastPath(calls)).toBe("F:\\Out\\Trip.mp4"));
    expect(vi.mocked(openDialog)).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true, multiple: false }),
    );
  });

  it("explains a custom bitrate Rust rejects", async () => {
    setup({
      checkExport: ({ settings }) => {
        if (settings.target.settings.quality.kind === "custom") {
          return rejectWith("invalidEdit", "kbps");
        }
        return result();
      },
    });
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: t.export.custom }));
    fireEvent.change(screen.getByLabelText(t.export.quality), { target: { value: "custom" } });
    fireEvent.change(screen.getByLabelText(t.export.customBitrate), { target: { value: "900" } });
    expect(await screen.findByText(t.export.bitrateRange)).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);
  });
});
