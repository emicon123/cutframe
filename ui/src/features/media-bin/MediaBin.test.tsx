import { mockWindows } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type MediaView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { initialImportState, useImportStore } from "@/stores/importStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";

import { MediaBin } from "./MediaBin";

const video: MediaView_Serialize = {
  id: "m1",
  kind: "video",
  fileName: "portrait.mp4",
  path: "C:\\clips\\portrait.mp4",
  duration: 42 * FLICKS_PER_SECOND,
  displayWidth: 1080,
  displayHeight: 1920,
  frameRate: { num: 30000, den: 1001 },
  videoCodec: "hevc",
  hdr: null,
  audio: null,
  status: "online",
  usedBy: 0,
  previewVersion: "0123456789abcdef",
};

function withMedia(media: MediaView_Serialize[]): ProjectState {
  return {
    revision: 1,
    project: {
      id: "p",
      name: "Untitled",
      settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
      duration: 0,
      tracks: [],
      media,
    },
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
    filePath: null,
    dirty: false,
  };
}

let calls: string[];

beforeEach(() => {
  calls = [];
  mockWindows("main");
  mockCommands({
    importMediaDialog: ({ filterLabel }) => {
      calls.push(`dialog ${filterLabel}`);
      return "job-1";
    },
    importMedia: ({ paths }) => {
      calls.push(`import ${paths.join(",")}`);
      return "job-2";
    },
    cancelJob: ({ id }) => {
      calls.push(`cancel ${id}`);
      return null;
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  useProjectStore.setState({ state: null });
  useImportStore.setState(initialImportState);
});

describe("MediaBin", () => {
  it("opens the import dialog with the translated filter label and tracks the job", async () => {
    render(<MediaBin />);
    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.import }));
    await waitFor(() => expect(calls).toEqual([`dialog ${t.mediaBin.filterLabel}`]));
    expect(await screen.findByRole("progressbar")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.job.cancel }));
    await waitFor(() => expect(calls).toContain("cancel job-1"));
    expect(screen.getByText(t.mediaBin.job.cancelling)).toBeTruthy();
  });

  it("lists imported media with kind, duration and resolution", () => {
    useProjectStore.setState({ state: withMedia([video]) });
    render(<MediaBin />);
    expect(screen.getByText("portrait.mp4")).toBeTruthy();
    expect(screen.getByText("Video · 0:42 · 1080×1920")).toBeTruthy();
  });

  it("shows translated per-file reasons after a job with failures", () => {
    useImportStore.getState().dispatch({
      type: "report",
      event: {
        jobId: "j",
        files: [
          { path: "C:\\clips\\a.mp4", outcome: { kind: "added", media: "m1" } },
          { path: "C:\\clips\\broken.mp4", outcome: { kind: "failed", error: "damaged", detail: "x" } },
        ],
      },
    });
    useImportStore.getState().dispatch({ type: "finished", event: { jobId: "j", result: { Ok: null } } });
    render(<MediaBin />);
    expect(screen.getByText("1 file imported")).toBeTruthy();
    expect(screen.getByText("1 file could not be imported")).toBeTruthy();
    expect(screen.getByText(`broken.mp4: ${t.mediaBin.errors.damaged}`)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.result.dismiss }));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("imports paths dropped from Explorer onto the panel, not elsewhere", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      DOMRect.fromRect({ x: 0, y: 0, width: 300, height: 400 }),
    );
    render(<MediaBin />);
    const drop = (x: number, paths: string[]) => emit("tauri://drag-drop", { paths, position: { x, y: 10 } });

    // The listener registers asynchronously; keep dropping outside until it is live, then inside.
    await waitFor(async () => {
      await drop(900, ["C:\\outside.mp4"]);
      await drop(10, ["C:\\clips\\a.mp4", "C:\\clips\\folder"]);
      expect(calls).toContain("import C:\\clips\\a.mp4,C:\\clips\\folder");
    });
    expect(calls.some((c) => c.includes("outside"))).toBe(false);
  });
});
