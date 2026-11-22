import { afterEach, describe, expect, it } from "vitest";

import type { AppInfo_Serialize, GpuInfoView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

import { applyAppInfoChange, gpuNoticeFor, gpuNoticeText, gpuWarningText } from "./gpuNotice";

const gpu = (overrides: Partial<GpuInfoView_Serialize> = {}): GpuInfoView_Serialize => ({
  adapter: "Radeon",
  backend: "Dx12",
  software: false,
  deviceType: "discrete",
  driver: "24.1",
  presentsToWindow: true,
  fallback: null,
  videoAcceleration: null,
  ...overrides,
});

const info = (overrides: Partial<AppInfo_Serialize> = {}): AppInfo_Serialize => ({
  version: "0.1.0",
  gpu: gpu(),
  previewMode: "native",
  ...overrides,
});

afterEach(() => useAppStore.setState({ info: null, gpuNotice: null }));

describe("gpuNoticeFor", () => {
  it("reports a restart after a device loss", () => {
    expect(gpuNoticeFor(info(), info({ gpu: gpu({ fallback: "deviceLost" }) }))).toEqual({
      kind: "gpuRestarted",
      adapter: "Radeon",
      software: false,
    });
  });

  it("reports a restart on software rendering", () => {
    const warp = gpu({ adapter: "Microsoft Basic Render Driver", software: true, fallback: "deviceLost" });
    const notice = gpuNoticeFor(info(), info({ gpu: warp }));
    expect(notice).toMatchObject({ kind: "gpuRestarted", software: true });
    expect(notice && gpuNoticeText(notice)).toBe(t.status.gpuRestartedSoftware);
  });

  it("reports the preview giving up once", () => {
    const gaveUp = info({ previewMode: "unavailable", gpu: gpu({ fallback: "deviceLost" }) });
    expect(gpuNoticeFor(info(), gaveUp)).toEqual({ kind: "previewLost" });
    expect(gpuNoticeFor(gaveUp, gaveUp)).toBeNull();
  });

  it("stays quiet for other changes and startup fallbacks", () => {
    expect(gpuNoticeFor(info(), info())).toBeNull();
    expect(gpuNoticeFor(info(), info({ gpu: gpu({ fallback: "forcedByEnv" }) }))).toBeNull();
  });
});

describe("gpuWarningText", () => {
  it("explains every fallback and nothing otherwise", () => {
    expect(gpuWarningText(info({ gpu: gpu({ fallback: "noHardwareAdapter" }) }))).toBe(
      t.gpu.fallbacks.noHardwareAdapter,
    );
    expect(gpuWarningText(info())).toBeNull();
    expect(gpuWarningText(info({ gpu: null }))).toBeNull();
    expect(gpuWarningText(null)).toBeNull();
  });

  it("names the adapter the preview restarted on", () => {
    expect(gpuNoticeText({ kind: "gpuRestarted", adapter: "Arc", software: false })).toBe(
      t.status.gpuRestarted("Arc"),
    );
  });
});

describe("applyAppInfoChange", () => {
  it("replaces the info and posts the notice", () => {
    useAppStore.setState({ info: info() });
    const lost = info({ gpu: gpu({ fallback: "deviceLost" }) });
    applyAppInfoChange(lost);
    expect(useAppStore.getState().info).toBe(lost);
    expect(useAppStore.getState().gpuNotice).toMatchObject({ kind: "gpuRestarted" });
  });

  it("keeps an earlier notice when the new info needs none", () => {
    useAppStore.setState({ info: info(), gpuNotice: { kind: "previewLost" } });
    applyAppInfoChange(info({ version: "0.1.1" }));
    expect(useAppStore.getState().gpuNotice).toEqual({ kind: "previewLost" });
    expect(useAppStore.getState().info?.version).toBe("0.1.1");
  });
});
