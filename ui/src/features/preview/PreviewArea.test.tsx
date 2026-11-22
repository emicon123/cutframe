import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockCommands, type PreviewRect } from "@/bindings";
import { useAppStore } from "@/stores/appStore";

import { PreviewArea } from "./PreviewArea";

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

let calls: string[];

beforeEach(() => {
  calls = [];
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  mockCommands({
    setPreviewRect: ({ rect }: { rect: PreviewRect }) => {
      calls.push(`rect ${rect.width}x${rect.height}`);
    },
    setPreviewVisible: ({ visible }) => {
      calls.push(`visible ${visible}`);
    },
  });
  useAppStore.setState({ info: { version: "0.0.0", gpu: null, previewMode: "native" } });
});

afterEach(() => {
  vi.unstubAllGlobals();
  useAppStore.setState({ info: null, previewOccluders: 0 });
});

describe("PreviewArea in native mode", () => {
  it("reports the rect, then requests visible on mount and hidden on unmount", async () => {
    const { unmount } = render(<PreviewArea />);
    await waitFor(() => expect(calls).toEqual(["rect 0x0", "visible true"]));

    unmount();
    await waitFor(() => expect(calls).toEqual(["rect 0x0", "visible true", "visible false"]));
  });

  it("hides the preview while an overlay occludes it", async () => {
    render(<PreviewArea />);
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
