/**
 * Preview quality (F1.61), zoom and pan (F1.62) and full screen (F1.63), render-graph.md §8. UI
 * state that the engine mirrors through fire-and-forget commands; nothing here is an edit.
 *
 * - Quality is a per-user UI preference in `localStorage` (like the panel layout and snapping,
 *   preferences design §6); Rust does not persist it, so it is sent at startup and on change.
 * - Zoom and pan last for the session; `fit` drops the pan.
 * - Full screen is the main window in full-screen mode with the preview area over the whole
 *   webview (ADR-0005 amendment §5); the native preview follows the reported rectangle.
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";

import { commands, type PreviewQuality, type PreviewZoom } from "@/bindings";
import { logError } from "@/ipc/errors";
import { logTransportError } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";

import {
  buildView,
  effectivePercent,
  FIT,
  fitPercent,
  NO_PAN,
  type Pan,
  panBy,
  panForZoom,
  type Size,
  stepZoom,
} from "./previewView";

export const QUALITIES: readonly PreviewQuality[] = ["full", "half", "quarter"];

const QUALITY_KEY = "preview.quality.v1";

function readQuality(): PreviewQuality {
  try {
    const stored = window.localStorage.getItem(QUALITY_KEY);
    return QUALITIES.find((q) => q === stored) ?? "full";
  } catch {
    return "full";
  }
}

function writeQuality(quality: PreviewQuality) {
  try {
    window.localStorage.setItem(QUALITY_KEY, quality);
  } catch {
    // Blocked storage: the choice lasts for this session only.
  }
}

type PreviewStore = {
  quality: PreviewQuality;
  zoom: PreviewZoom;
  /** Canvas fractions from the centre; always zero at `fit`. */
  pan: Pan;
  fullScreen: boolean;
};

export const usePreviewStore = create<PreviewStore>()(() => ({
  quality: readQuality(),
  zoom: FIT,
  pan: NO_PAN,
  fullScreen: false,
}));

/** Size of the preview area in physical pixels, as last reported with `setPreviewRect`. */
let areaSize: Size | null = null;

export function reportPreviewAreaSize(size: Size): void {
  areaSize = size;
}

function canvasSize(): Size | null {
  const settings = useProjectStore.getState().state?.project.settings;
  return settings ? { width: settings.width, height: settings.height } : null;
}

/** The percentage `fit` stands for right now, or `null` while the area or canvas is unknown. */
export function currentFitPercent(): number | null {
  return fitPercent(areaSize, canvasSize());
}

function sendQuality(quality: PreviewQuality) {
  commands.setPreviewQuality(quality).catch(logTransportError("setPreviewQuality"));
}

function sendView() {
  const { zoom, pan } = usePreviewStore.getState();
  commands.setPreviewView(buildView(zoom, pan)).catch(logTransportError("setPreviewView"));
}

/**
 * Startup (and after a webview reload, when the engine still has the old values): the stored
 * quality and the session's view.
 */
export function syncPreviewSettings(): void {
  sendQuality(usePreviewStore.getState().quality);
  sendView();
}

export function setPreviewQuality(quality: PreviewQuality): void {
  writeQuality(quality);
  usePreviewStore.setState({ quality });
  sendQuality(quality);
}

/** A zoom level from the menu, or `fit` (which drops the pan). */
export function setPreviewZoom(zoom: PreviewZoom): void {
  const { zoom: before, pan } = usePreviewStore.getState();
  // From `fit` the picture is centred; keep the pan between percentages.
  const nextPan = zoom.kind === "fit" || before.kind === "fit" ? NO_PAN : pan;
  usePreviewStore.setState({ zoom, pan: nextPan });
  sendView();
}

/**
 * One zoom step in (`1`) or out (`-1`). With `offset` (physical pixels from the area's centre,
 * Ctrl+wheel) the point under the pointer stays in place; otherwise the centre does.
 */
export function zoomPreview(direction: -1 | 1, offset?: { x: number; y: number }): void {
  const { zoom, pan } = usePreviewStore.getState();
  const fit = currentFitPercent();
  const next = stepZoom(zoom, direction, fit);
  const canvas = canvasSize();
  const from = effectivePercent(zoom, fit);
  const start = zoom.kind === "fit" ? NO_PAN : pan;
  const nextPan =
    offset && canvas && next.kind === "percent"
      ? panForZoom(start, offset, canvas, from, next.percent)
      : start;
  usePreviewStore.setState({ zoom: next, pan: nextPan });
  sendView();
}

let panFrame: number | null = null;

/**
 * Drags the zoomed picture: `start` is the pan when the drag began, (`dx`, `dy`) the pointer travel
 * in physical pixels. The store follows at once; `setPreviewView` goes out at most once per
 * animation frame with the latest pan. Ignored at `fit`.
 */
export function dragPreviewPan(start: Pan, dx: number, dy: number): void {
  const { zoom } = usePreviewStore.getState();
  const canvas = canvasSize();
  if (zoom.kind === "fit" || !canvas) return;
  usePreviewStore.setState({ pan: panBy(start, dx, dy, canvas, zoom.percent) });
  panFrame ??= requestAnimationFrame(() => {
    panFrame = null;
    sendView();
  });
}

function applyWindowFullScreen(on: boolean): void {
  getCurrentWindow()
    .setFullscreen(on)
    .catch((error: unknown) => {
      logError(error);
      // The window did not change: show the layout that matches it.
      usePreviewStore.setState({ fullScreen: !on });
    });
}

/** F: the main window full screen with only the preview area, or back to the editor layout. */
export function toggleFullScreen(): void {
  const fullScreen = !usePreviewStore.getState().fullScreen;
  usePreviewStore.setState({ fullScreen });
  applyWindowFullScreen(fullScreen);
}

/** Esc in full screen, and leaving the editor. Does nothing when not full screen. */
export function exitFullScreen(): void {
  if (!usePreviewStore.getState().fullScreen) return;
  usePreviewStore.setState({ fullScreen: false });
  applyWindowFullScreen(false);
}
