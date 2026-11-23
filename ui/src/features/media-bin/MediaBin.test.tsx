import { mockConvertFileSrc, mockWindows } from "@tauri-apps/api/mocks";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type MediaView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { jobResults } from "@/ipc/jobResults";
import { useAppStore } from "@/stores/appStore";
import { initialImportState, useImportStore } from "@/stores/importStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { MediaBin } from "./MediaBin";
import { closeRelink } from "./relinkStore";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

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
  filmstrip: { interval: FLICKS_PER_SECOND, frames: 42, framesPerTile: 32, frameWidth: 54 },
};

const photo: MediaView_Serialize = {
  ...video,
  id: "m2",
  kind: "image",
  fileName: "beach.heic",
  path: "C:\\photos\\beach.heic",
  duration: null,
  hdr: "hlg",
  previewVersion: "fedcba9876543210",
  filmstrip: null,
};

const song: MediaView_Serialize = {
  ...video,
  id: "m3",
  kind: "audio",
  fileName: "song.mp3",
  path: "C:\\music\\song.mp3",
  duration: 185 * FLICKS_PER_SECOND,
  displayWidth: null,
  displayHeight: null,
  previewVersion: "aaaaaaaaaaaaaaaa",
  filmstrip: null,
};

const option = (name: string) => screen.getByRole("option", { name: new RegExp(name.replace(".", "\\.")) });

function withMedia(media: MediaView_Serialize[]): ProjectState {
  return {
    revision: 1,
    project: {
      id: "p",
      name: "Untitled",
      settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
      duration: 0,
      background: { kind: "color", color: "#000000ff" },
      mainTrack: "main",
      magnetic: true,
      tracks: [],
      media,
    },
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null, entries: [], current: 0 },
    filePath: null,
    dirty: false,
  };
}

let calls: string[];

beforeEach(() => {
  calls = [];
  mockWindows("main");
  mockConvertFileSrc("windows");
  // Waveform requests never answer in these tests.
  vi.stubGlobal("fetch", () => new Promise(() => undefined));
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
    removeMedia: ({ ids }) => {
      calls.push(`remove ${ids.join(",")}`);
      return null;
    },
    revealMedia: ({ id }) => {
      calls.push(`reveal ${id}`);
      return null;
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.mocked(openDialog).mockReset();
  closeRelink();
  vi.unstubAllGlobals();
  useUiStore.setState({
    mediaLayout: "grid",
    mediaSort: "imported",
    mediaSortDirection: "asc",
    mediaKinds: [],
  });
  useAppStore.setState({ previewOccluders: 0 });
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

  it("shows a grid with posters, durations and an HDR badge, and a list view with details", () => {
    useProjectStore.setState({ state: withMedia([video, photo, song]) });
    render(<MediaBin />);
    const tile = option("portrait.mp4");
    expect(within(tile).getByText("0:42")).toBeTruthy();
    expect(tile.querySelector("img")?.getAttribute("src")).toBe(
      "http://media.localhost/poster/m1?v=0123456789abcdef",
    );
    expect(tile.querySelector("img")?.getAttribute("loading")).toBe("lazy");
    expect(within(option("beach.heic")).getByText(t.mediaBin.hdr.badge.hlg)).toBeTruthy();
    // Audio items draw a waveform instead of a poster.
    expect(option("song.mp3").querySelector("img")).toBeNull();
    expect(option("song.mp3").querySelector("canvas")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.layout.list }));
    expect(useUiStore.getState().mediaLayout).toBe("list");
    expect(screen.getByText("Video · 0:42 · 1080×1920")).toBeTruthy();
  });

  it("searches by name and sorts by the remembered key", async () => {
    useProjectStore.setState({ state: withMedia([video, photo, song]) });
    useUiStore.setState({ mediaSort: "name" });
    render(<MediaBin />);
    const names = () => screen.getAllByRole("option").map((o) => o.querySelector("p")?.textContent);
    expect(names()).toEqual(["beach.heic", "portrait.mp4", "song.mp3"]);

    fireEvent.change(screen.getByRole("searchbox", { name: t.mediaBin.search }), {
      target: { value: "SON" },
    });
    await waitFor(() => expect(names()).toEqual(["song.mp3"]));

    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "zzz" } });
    expect(await screen.findByText(t.mediaBin.noMatches)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.clearFilters }));
    await waitFor(() => expect(names()).toHaveLength(3));
  });

  it("removes the selection after confirmation with Delete", async () => {
    useProjectStore.setState({ state: withMedia([video, photo, song]) });
    render(<MediaBin />);
    fireEvent.click(option("portrait.mp4"));
    fireEvent.click(option("song.mp3"), { ctrlKey: true });
    expect(option("portrait.mp4").getAttribute("aria-selected")).toBe("true");
    expect(option("beach.heic").getAttribute("aria-selected")).toBe("false");

    fireEvent.keyDown(option("song.mp3"), { key: "Delete" });
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(t.mediaBin.remove.title(2, ""))).toBeTruthy();
    expect(useAppStore.getState().previewOccluders).toBe(1);
    fireEvent.click(within(dialog).getByRole("button", { name: t.mediaBin.remove.confirm }));
    await waitFor(() => expect(calls).toEqual(["remove m1,m3"]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("moves the selection and focus with arrow keys and extends it with Shift", () => {
    useProjectStore.setState({ state: withMedia([video, photo, song]) });
    useUiStore.setState({ mediaLayout: "list" });
    render(<MediaBin />);
    fireEvent.click(option("portrait.mp4"));
    fireEvent.keyDown(option("portrait.mp4"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(option("beach.heic"));
    fireEvent.keyDown(option("beach.heic"), { key: "ArrowDown", shiftKey: true });
    const selected = screen.getAllByRole("option").filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected.map((o) => o.dataset.mediaId)).toEqual(["m2", "m3"]);
    expect(option("song.mp3").tabIndex).toBe(0);
    expect(option("portrait.mp4").tabIndex).toBe(-1);
  });

  it("cancelling the confirmation removes nothing", async () => {
    useProjectStore.setState({ state: withMedia([video]) });
    render(<MediaBin />);
    fireEvent.click(option("portrait.mp4"));
    fireEvent.keyDown(option("portrait.mp4"), { key: "Delete" });
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(t.mediaBin.remove.title(1, "portrait.mp4"))).toBeTruthy();
    // The footer button; the dialog's ✕ carries the same label.
    const [cancel] = within(dialog).getAllByRole("button", { name: t.mediaBin.remove.cancel });
    if (cancel) fireEvent.click(cancel);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls).toEqual([]);
  });

  it("reveals the right-clicked item in Explorer from the context menu", async () => {
    useProjectStore.setState({ state: withMedia([video, photo]) });
    render(<MediaBin />);
    fireEvent.contextMenu(option("beach.heic"));
    const menu = await screen.findByRole("menu");
    // The open menu hides the rest of the page from the accessibility tree.
    const tile = screen.getByRole("option", { name: /beach/, hidden: true });
    expect(tile.getAttribute("aria-selected")).toBe("true");
    expect(useAppStore.getState().previewOccluders).toBe(1);
    fireEvent.click(within(menu).getByRole("menuitem", { name: t.mediaBin.actions.reveal }));
    await waitFor(() => expect(calls).toEqual(["reveal m2"]));
  });

  it("skims a video: the pointer position picks the filmstrip tile and frame", () => {
    useProjectStore.setState({ state: withMedia([video]) });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      DOMRect.fromRect({ x: 0, y: 0, width: 100, height: 56 }),
    );
    render(<MediaBin />);
    const skim = within(option("portrait.mp4")).getByTestId("skim");
    // 42 frames; x = 80 % lands on frame 33, slot 1 of tile 1 (which holds 10 frames).
    fireEvent.pointerMove(skim, { clientX: 80, pointerType: "mouse" });
    const sprite = skim.firstElementChild as HTMLElement;
    expect(sprite.style.backgroundImage).toContain("filmstrip/m1?tile=1&v=0123456789abcdef");
    expect(sprite.style.backgroundSize).toBe("1000% 100%");
    fireEvent.pointerLeave(skim);
    expect(skim.childElementCount).toBe(0);
  });

  it("shows background preview generation without a cancel button or result", () => {
    useProjectStore.setState({ state: withMedia([video]) });
    useImportStore.getState().dispatch({
      type: "progress",
      event: { jobId: "bg", stage: "generatingPreviews", progress: 0.25 },
    });
    render(<MediaBin />);
    const bar = screen.getByRole("progressbar", { name: t.mediaBin.background });
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.queryByRole("button", { name: t.mediaBin.job.cancel })).toBeNull();
  });

  it("shows translated per-file reasons after a job with failures", () => {
    useImportStore.getState().dispatch({ type: "started", id: "j" });
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

  it("flags missing media and relinks it after confirming a content mismatch", async () => {
    const missing: MediaView_Serialize = { ...video, status: "offline" };
    useProjectStore.setState({ state: withMedia([missing, photo]) });
    vi.mocked(openDialog).mockResolvedValue("D:\\moved\\portrait.mp4");
    let job = 0;
    mockCommands({
      relinkMedia: ({ id, path, allowMismatch }) => {
        calls.push(`relink ${id} ${path} ${allowMismatch}`);
        const jobId = `r${++job}`;
        const outcome = allowMismatch
          ? ({ kind: "relinked", media: id } as const)
          : ({ kind: "fingerprintMismatch", media: id } as const);
        // The job may finish before its command resolves; jobResults keeps the result.
        jobResults.report({ jobId, files: [{ path, outcome }] });
        jobResults.finished({ jobId, result: { Ok: null } });
        return jobId;
      },
    });
    render(<MediaBin />);
    expect(within(option("portrait.mp4")).getByText(t.mediaBin.status.badge.offline)).toBeTruthy();
    const notice = screen.getByText(t.mediaBin.notice.text(1, 0));
    expect(notice.textContent).toBe("1 file missing");

    fireEvent.click(screen.getByRole("button", { name: t.mediaBin.notice.relink }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(t.mediaBin.relink.titleAll)).toBeTruthy();
    // Only the item that needs a relink is listed.
    expect(within(dialog).queryByText("beach.heic")).toBeNull();
    fireEvent.click(
      within(dialog).getByRole("button", { name: t.mediaBin.relink.locateItem("portrait.mp4") }),
    );
    await waitFor(() => expect(calls).toEqual(["relink m1 D:\\moved\\portrait.mp4 false"]));
    expect(vi.mocked(openDialog).mock.calls[0]?.[0]).toMatchObject({
      defaultPath: "C:\\clips",
      directory: false,
    });

    const alert = await within(dialog).findByRole("alert");
    expect(within(alert).getByText(t.mediaBin.relink.mismatch)).toBeTruthy();
    fireEvent.click(within(alert).getByRole("button", { name: t.mediaBin.relink.useAnyway }));
    await waitFor(() => expect(calls[1]).toBe("relink m1 D:\\moved\\portrait.mp4 true"));
    expect(await within(dialog).findByText(t.mediaBin.relink.outcomes.relinked(1))).toBeTruthy();
    expect(within(dialog).queryByRole("alert")).toBeNull();
  });

  it("searches a folder and lists the files it did not find", async () => {
    const a: MediaView_Serialize = { ...video, status: "offline" };
    const b: MediaView_Serialize = { ...song, status: "offline" };
    useProjectStore.setState({ state: withMedia([a, b]) });
    vi.mocked(openDialog).mockResolvedValue("D:\\moved");
    mockCommands({
      relinkMediaFolder: ({ folder }) => {
        calls.push(`folder ${folder}`);
        return new Promise((resolve) => {
          resolve("f1");
          setTimeout(() => {
            jobResults.report({
              jobId: "f1",
              files: [
                { path: "D:\\moved\\portrait.mp4", outcome: { kind: "relinked", media: "m1" } },
                { path: "C:\\music\\song.mp3", outcome: { kind: "notFound", media: "m3" } },
              ],
            });
            jobResults.finished({ jobId: "f1", result: { Ok: null } });
          }, 10);
        });
      },
    });
    render(<MediaBin />);
    fireEvent.contextMenu(option("song.mp3"));
    fireEvent.click(await screen.findByRole("menuitem", { name: t.mediaBin.relink.action }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(t.mediaBin.relink.titleOne("song.mp3"))).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: t.mediaBin.relink.searchFolder }));
    await waitFor(() => expect(calls).toEqual(["folder D:\\moved"]));
    const status = await within(dialog).findByText(t.mediaBin.relink.outcomes.notFound(1));
    expect(status).toBeTruthy();
    expect(within(dialog).getByText(t.mediaBin.relink.outcomes.relinked(1))).toBeTruthy();
  });
});
