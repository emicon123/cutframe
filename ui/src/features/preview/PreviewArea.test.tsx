import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type MockCommandHandlers, type PreviewRect } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";
import { useAppStore } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { PreviewArea } from "./PreviewArea";
import { usePreviewStore } from "./previewStore";

const setFullscreen = vi.fn<(on: boolean) => Promise<void>>(() => Promise.resolve());
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFullscreen }) }));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

let calls: string[];

const renderPreview = () =>
  render(
    <TooltipProvider>
      <PreviewArea />
    </TooltipProvider>,
  );

function mockPreview(extra: Partial<MockCommandHandlers> = {}) {
  mockCommands({
    ...extra,
    setPreviewRect: ({ rect }: { rect: PreviewRect }) => {
      calls.push(`rect ${rect.width}x${rect.height}`);
    },
    setPreviewVisible: ({ visible }) => {
      calls.push(`visible ${visible}`);
    },
  });
}

beforeEach(() => {
  calls = [];
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  mockPreview();
  useAppStore.setState({ info: { version: "0.0.0", gpu: null, previewMode: "native" } });
});

afterEach(() => {
  vi.unstubAllGlobals();
  useAppStore.setState({ info: null, previewOccluders: 0 });
  useProjectStore.setState({ state: null });
  useUiStore.setState({ view: "home" });
  playheadStore.setState({
    time: 0,
    state: "paused",
    rate: 0,
    looping: false,
    seekGeneration: null,
    source: null,
  });
  usePreviewStore.setState({
    fullScreen: false,
    zoom: { kind: "fit" },
    pan: { x: 0, y: 0 },
    quality: "full",
  });
});

describe("PreviewArea in native mode", () => {
  it("reports the rect, then requests visible on mount and hidden on unmount", async () => {
    const { unmount } = renderPreview();
    await waitFor(() => expect(calls).toEqual(["rect 0x0", "visible true"]));

    unmount();
    await waitFor(() => expect(calls).toEqual(["rect 0x0", "visible true", "visible false"]));
  });

  it("hides the preview while an overlay occludes it", async () => {
    renderPreview();
    await waitFor(() => expect(calls).toEqual(["rect 0x0", "visible true"]));

    act(() => useAppStore.getState().addPreviewOccluder());
    act(() => useAppStore.getState().addPreviewOccluder());
    await waitFor(() => expect(calls.at(-1)).toBe("visible false"));
    act(() => useAppStore.getState().removePreviewOccluder());
    expect(calls.at(-1)).toBe("visible false");
    act(() => useAppStore.getState().removePreviewOccluder());
    await waitFor(() => expect(calls.at(-1)).toBe("visible true"));
    expect(calls).toEqual(["rect 0x0", "visible true", "visible false", "visible true"]);
  });
});

describe("TransportBar (F1.59, F1.67)", () => {
  const S = FLICKS_PER_SECOND;

  function openProject(duration: number) {
    useProjectStore.setState({
      state: {
        project: { duration, magnetic: true, settings: { frameRate: { num: 25, den: 1 } }, tracks: [] },
        history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
      } as unknown as ProjectState,
    });
    useUiStore.setState({ view: "editor" });
  }

  it("shows the playhead and the timeline length, following the playhead store", () => {
    openProject(65 * S);
    playheadStore.setState({ time: 2 * S + (3 * S) / 25 });
    renderPreview();
    expect(screen.getByTestId("playhead-timecode").textContent).toBe("00:00:02:03");
    expect(screen.getByTestId("timeline-length").textContent).toBe("00:01:05:00");
    act(() => playheadStore.setState({ time: 10 * S }));
    expect(screen.getByTestId("playhead-timecode").textContent).toBe("00:00:10:00");
  });

  it("plays and pauses through togglePlayback and shows the transport state", async () => {
    const sent: string[] = [];
    mockPreview({
      togglePlayback: () => {
        sent.push("toggle");
        return null;
      },
    });
    openProject(10 * S);
    renderPreview();

    fireEvent.click(screen.getByRole("button", { name: t.playback.play }));
    await waitFor(() => expect(sent).toEqual(["toggle"]));
    act(() => playheadStore.setState({ state: "playing", rate: 1 }));
    expect(screen.getByRole("button", { name: t.playback.pause })).toBeDefined();
    act(() => playheadStore.setState({ state: "paused", rate: 0 }));
    expect(screen.getByRole("button", { name: t.playback.play })).toBeDefined();
  });

  it("disables Play on an empty timeline", () => {
    openProject(0);
    renderPreview();
    expect(screen.getByRole("button", { name: t.playback.play }).hasAttribute("disabled")).toBe(true);
  });

  it("toggles loop playback and reflects the engine's loop flag", async () => {
    const loops: boolean[] = [];
    mockPreview({
      setLoop: ({ on }) => {
        loops.push(on);
      },
    });
    openProject(10 * S);
    renderPreview();
    const button = screen.getByRole("button", { name: t.playback.loop });
    expect(button.getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(button);
    await waitFor(() => expect(loops).toEqual([true]));
    act(() => playheadStore.setState({ looping: true }));
    expect(button.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(button);
    await waitFor(() => expect(loops).toEqual([true, false]));
  });

  it("logs a transport command without a preview engine at debug level only", async () => {
    const debug = vi.spyOn(console, "debug").mockImplementation(() => undefined);
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mockPreview({
      // Tauri rejects a failed command with the serialized `IpcError` itself, not an `Error`.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
      togglePlayback: () => Promise.reject({ kind: "unavailable", message: "no preview engine" }),
    });
    openProject(10 * S);
    renderPreview();
    fireEvent.click(screen.getByRole("button", { name: t.playback.play }));
    await waitFor(() => expect(debug).toHaveBeenCalledWith("[ipc] togglePlayback: no preview engine"));
    expect(error).not.toHaveBeenCalled();
  });
});

describe("source preview (F1.17)", () => {
  const S = FLICKS_PER_SECOND;

  function openProject() {
    useProjectStore.setState({
      state: {
        project: {
          duration: 65 * S,
          magnetic: true,
          settings: { width: 1920, height: 1080, frameRate: { num: 25, den: 1 } },
          tracks: [],
          media: [{ id: "m1", fileName: "beach.mp4", duration: 8 * S, frameRate: { num: 50, den: 1 } }],
        },
        history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
      } as unknown as ProjectState,
    });
    useUiStore.setState({ view: "editor" });
  }

  it("names the media, shows media time and length, and leads back to the timeline", async () => {
    const targets: string[] = [];
    mockPreview({
      setPlaybackTarget: ({ target }) => {
        targets.push(target.kind);
        return 7;
      },
    });
    openProject();
    playheadStore.setState({ time: 10 * S });
    renderPreview();
    expect(screen.queryByText(t.preview.source.previewing("beach.mp4"))).toBeNull();

    act(() =>
      playheadStore.setState({ source: { media: "m1", time: S + S / 50, at: 0, state: "paused", rate: 0 } }),
    );
    expect(screen.getByText(t.preview.source.previewing("beach.mp4"))).toBeDefined();
    expect(screen.getByTestId("playhead-timecode").textContent).toBe("00:00:01:01");
    expect(screen.getByTestId("timeline-length").textContent).toBe("00:00:08:00");

    fireEvent.click(screen.getByRole("button", { name: t.playback.backToTimeline }));
    await waitFor(() => expect(targets).toEqual(["timeline"]));
    expect(screen.queryByText(t.preview.source.previewing("beach.mp4"))).toBeNull();
    expect(screen.getByTestId("playhead-timecode").textContent).toBe("00:00:10:00");
    expect(screen.getByTestId("timeline-length").textContent).toBe("00:01:05:00");
  });
});

describe("preview controls (F1.61-F1.63)", () => {
  function openProject() {
    useProjectStore.setState({
      state: {
        project: {
          duration: FLICKS_PER_SECOND,
          magnetic: true,
          settings: { width: 1920, height: 1080, frameRate: { num: 25, den: 1 } },
          tracks: [],
          media: [],
        },
        history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
      } as unknown as ProjectState,
    });
    useUiStore.setState({ view: "editor" });
  }

  it("covers the whole webview in full screen and restores the panel on exit", async () => {
    openProject();
    renderPreview();
    const area = screen.getByTestId("preview-area");
    expect(area.className).not.toContain("fixed");

    fireEvent.click(screen.getByRole("button", { name: t.view.fullScreen }));
    expect(area.className).toContain("fixed");
    expect(setFullscreen).toHaveBeenLastCalledWith(true);

    fireEvent.click(screen.getByRole("button", { name: t.view.exitFullScreen }));
    expect(area.className).not.toContain("fixed");
    await waitFor(() => expect(setFullscreen).toHaveBeenLastCalledWith(false));
  });

  it("zooms with Ctrl+wheel over the picture, one step per notch", async () => {
    const views: string[] = [];
    mockPreview({
      setPreviewView: ({ view }) => {
        views.push(view.zoom.kind === "fit" ? "fit" : `${view.zoom.percent}`);
      },
    });
    openProject();
    renderPreview();
    const picture = screen.getByTestId("preview-area").children[0];
    if (!picture) throw new Error("no picture");
    const wheel = (deltaY: number, ctrlKey = true) =>
      picture.dispatchEvent(new WheelEvent("wheel", { deltaY, ctrlKey, bubbles: true, cancelable: true }));

    expect(wheel(-100, false)).toBe(true);
    expect(wheel(-100)).toBe(false); // default prevented: no page zoom
    wheel(-20);
    wheel(-40);
    // The slot is 0 x 0 in jsdom, so fit counts as 100 %.
    await waitFor(() => expect(views).toEqual(["200", "400"]));
    expect(usePreviewStore.getState().zoom).toEqual({ kind: "percent", percent: 400 });
  });
});
