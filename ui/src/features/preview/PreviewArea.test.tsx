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
  playheadStore.setState({ time: 0, state: "paused", rate: 0, looping: false, seekGeneration: null });
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
