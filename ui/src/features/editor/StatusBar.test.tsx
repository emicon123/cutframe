import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";

import { StatusBar } from "./StatusBar";

const renderBar = () =>
  render(
    <TooltipProvider>
      <StatusBar />
    </TooltipProvider>,
  );

afterEach(() => {
  useAppStore.setState({ info: null, gpuNotice: null });
  playheadStore.setState({ state: "paused", rate: 0, droppedFrames: 0 });
});

describe("StatusBar", () => {
  it("shows a warning icon explaining a fallback", () => {
    useAppStore.setState({
      info: {
        version: "0.1.0",
        previewMode: "native",
        gpu: {
          adapter: "WARP",
          backend: "Dx12",
          software: true,
          deviceType: "software",
          driver: "",
          presentsToWindow: true,
          fallback: "forcedByEnv",
          videoAcceleration: null,
        },
      },
    });
    renderBar();
    expect(screen.getByRole("button", { name: t.gpu.fallbacks.forcedByEnv })).toBeDefined();
  });

  it("shows the GPU notice until dismissed", () => {
    useAppStore.setState({ gpuNotice: { kind: "gpuRestarted", adapter: "Arc", software: false } });
    renderBar();
    expect(screen.getByRole("status").textContent).toContain(t.status.gpuRestarted("Arc"));
    fireEvent.click(screen.getByRole("button", { name: t.status.dismiss }));
    expect(screen.queryByRole("status")).toBeNull();
    expect(useAppStore.getState().gpuNotice).toBeNull();
  });

  it("shows dropped frames only while playing and only when there are some (F1.60)", () => {
    renderBar();
    expect(screen.queryByTestId("dropped-frames")).toBeNull();
    act(() => playheadStore.setState({ state: "playing", rate: 1, droppedFrames: 0 }));
    expect(screen.queryByTestId("dropped-frames")).toBeNull();
    act(() => playheadStore.setState({ droppedFrames: 3 }));
    expect(screen.getByTestId("dropped-frames").textContent).toBe(t.status.droppedFrames(3));
    act(() => playheadStore.setState({ state: "paused", rate: 0 }));
    expect(screen.queryByTestId("dropped-frames")).toBeNull();
  });
});
