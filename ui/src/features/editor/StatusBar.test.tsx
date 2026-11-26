import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

import { StatusBar } from "./StatusBar";

const renderBar = () =>
  render(
    <TooltipProvider>
      <StatusBar />
    </TooltipProvider>,
  );

afterEach(() => useAppStore.setState({ info: null, gpuNotice: null }));

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
});
