import { ArrowLeft, ChevronDown, Maximize, Minimize, Pause, Play, Repeat } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import { commands, type FrameRate, type MediaId, type PreviewZoom } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  type ActionId,
  actionLabel,
  actionTooltip,
  getAction,
  isEnabled,
  shortcutLabel,
  useActionContext,
} from "@/features/editor/actions";
import { t } from "@/i18n";
import { mediaUrl } from "@/ipc/media";
import { formatTimecode } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";

import { usePreviewOccluder } from "./occlusion";
import {
  dragPreviewPan,
  QUALITIES,
  reportPreviewAreaSize,
  setPreviewQuality,
  setPreviewZoom,
  usePreviewStore,
  zoomPreview,
} from "./previewStore";
import { FIT, type Pan, ZOOM_LEVELS } from "./previewView";

/** Frames mode only: upper bound on how often the UI pulls a frame. */
const FRAME_INTERVAL_MS = 1000 / 30;

/** Ctrl+wheel travel (CSS px of `deltaY`) per zoom step; one mouse notch is about 100. */
const WHEEL_STEP = 50;

/**
 * The preview panel: the picture, a source-preview bar and the transport bar. In full screen
 * (F1.63) it covers the whole webview above the editor panels, which stay mounted underneath, so
 * leaving full screen shows the layout exactly as it was; the native preview follows the slot's
 * new rectangle.
 */
export function PreviewArea() {
  const mode = useAppStore((s) => s.info?.previewMode);
  const fullScreen = usePreviewStore((s) => s.fullScreen);
  const pictureRef = useRef<HTMLDivElement>(null);
  usePreviewViewInput(pictureRef);

  return (
    <div
      className={cn("flex h-full flex-col bg-panel", fullScreen && "fixed inset-0 z-40")}
      data-testid="preview-area"
      data-full-screen={fullScreen || undefined}
    >
      <SourceBar />
      <div ref={pictureRef} className="relative min-h-0 flex-1 bg-preview-bg">
        {mode === "native" && <NativePreviewSlot />}
        {mode === "frames" && <FramePreview />}
        {mode === "unavailable" && <PreviewMessage text={t.preview.unavailable} />}
        {mode === undefined && <PreviewMessage text={t.preview.loading} />}
      </div>
      <TransportBar />
    </div>
  );
}

function PreviewMessage({ text }: { text: string }) {
  return <div className="flex size-full items-center justify-center text-muted-foreground">{text}</div>;
}

/**
 * F1.62: Ctrl+wheel over the picture zooms one step per notch around the pointer; dragging
 * (left or middle button) pans while zoomed in. The native preview lets pointer input through to
 * the webview (`HTTRANSPARENT`), so the DOM under it receives these events.
 */
function usePreviewViewInput(ref: RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    let wheel = 0;
    let drag: { id: number; x: number; y: number; pan: Pan } | null = null;

    const zoomed = () => usePreviewStore.getState().zoom.kind === "percent";
    const setCursor = (cursor: string) => {
      if (element.style.cursor !== cursor) element.style.cursor = cursor;
    };
    setCursor(zoomed() ? "grab" : "");
    const unsubscribe = usePreviewStore.subscribe((s) => {
      if (!drag) setCursor(s.zoom.kind === "percent" ? "grab" : "");
    });

    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      wheel += event.deltaY;
      if (Math.abs(wheel) < WHEEL_STEP) return;
      const direction = wheel < 0 ? 1 : -1;
      wheel = 0;
      const box = element.getBoundingClientRect();
      const dpr = window.devicePixelRatio;
      zoomPreview(direction, {
        x: (event.clientX - box.left - box.width / 2) * dpr,
        y: (event.clientY - box.top - box.height / 2) * dpr,
      });
    };
    const onPointerDown = (event: PointerEvent) => {
      if ((event.button !== 0 && event.button !== 1) || !zoomed() || drag) return;
      // The middle button would start the WebView's auto-scroll.
      event.preventDefault();
      element.setPointerCapture(event.pointerId);
      drag = { id: event.pointerId, x: event.clientX, y: event.clientY, pan: usePreviewStore.getState().pan };
      setCursor("grabbing");
    };
    const onPointerMove = (event: PointerEvent) => {
      if (!drag || event.pointerId !== drag.id) return;
      const dpr = window.devicePixelRatio;
      dragPreviewPan(drag.pan, (event.clientX - drag.x) * dpr, (event.clientY - drag.y) * dpr);
    };
    const onPointerEnd = (event: PointerEvent) => {
      if (!drag || event.pointerId !== drag.id) return;
      drag = null;
      setCursor(zoomed() ? "grab" : "");
    };

    element.addEventListener("wheel", onWheel, { passive: false });
    element.addEventListener("pointerdown", onPointerDown);
    element.addEventListener("pointermove", onPointerMove);
    element.addEventListener("pointerup", onPointerEnd);
    element.addEventListener("pointercancel", onPointerEnd);
    element.addEventListener("lostpointercapture", onPointerEnd);
    return () => {
      unsubscribe();
      element.removeEventListener("wheel", onWheel);
      element.removeEventListener("pointerdown", onPointerDown);
      element.removeEventListener("pointermove", onPointerMove);
      element.removeEventListener("pointerup", onPointerEnd);
      element.removeEventListener("pointercancel", onPointerEnd);
      element.removeEventListener("lostpointercapture", onPointerEnd);
    };
  }, [ref]);
}

/**
 * Windows: an empty slot. A native GPU window sits on top of it, so nothing in the DOM may be
 * drawn here. We report where the slot is, in physical pixels, and request visibility. The native
 * preview starts hidden and shows only while visibility is requested and the rect is non-empty
 * (ADR-0005 §3), so mounting reports the rect first, then requests visible; unmounting hides it.
 * This is the only place that calls `setPreviewVisible`: overlays register through
 * `usePreviewOccluder` and the preview is hidden while any is open.
 */
function NativePreviewSlot() {
  const ref = useRef<HTMLDivElement>(null);
  const occluded = useAppStore((s) => s.previewOccluders > 0);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const report = () => {
      const dpr = window.devicePixelRatio;
      const box = element.getBoundingClientRect();
      const rect = {
        x: Math.round(box.left * dpr),
        y: Math.round(box.top * dpr),
        width: Math.max(0, Math.round(box.width * dpr)),
        height: Math.max(0, Math.round(box.height * dpr)),
      };
      reportPreviewAreaSize(rect);
      void commands.setPreviewRect(rect);
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    // Panel resizes move the slot without changing its size; window resizes (also entering and
    // leaving full screen) change DPR or layout.
    window.addEventListener("resize", report);
    report();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", report);
      void commands.setPreviewVisible(false);
    };
  }, []);

  // Declared after the rect effect, so on mount the rect is reported before visibility.
  useEffect(() => {
    void commands.setPreviewVisible(!occluded);
  }, [occluded]);

  return <div ref={ref} className="absolute inset-0" />;
}

/** Linux development mode: pulls rendered frames over `media://` and paints them. */
function FramePreview() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    let stopped = false;
    let timer = 0;

    const pull = async () => {
      if (stopped) return;
      const started = performance.now();
      const dpr = window.devicePixelRatio;
      const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
      const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
      try {
        const response = await fetch(mediaUrl(`preview/frame?w=${width}&h=${height}`));
        if (response.ok) {
          const buffer = await response.arrayBuffer();
          const header = new DataView(buffer, 0, 8);
          const frameWidth = header.getUint32(0, true);
          const frameHeight = header.getUint32(4, true);
          const pixels = new Uint8ClampedArray(buffer, 8, frameWidth * frameHeight * 4);
          if (canvas.width !== frameWidth || canvas.height !== frameHeight) {
            canvas.width = frameWidth;
            canvas.height = frameHeight;
          }
          ctx.putImageData(new ImageData(pixels, frameWidth, frameHeight), 0, 0);
        }
      } catch {
        // The next pull retries; a dev-only preview should not spam errors.
      }
      const wait = Math.max(0, FRAME_INTERVAL_MS - (performance.now() - started));
      timer = window.setTimeout(() => void pull(), document.hidden ? 500 : wait);
    };
    void pull();

    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, []);

  return <canvas ref={canvasRef} className="absolute inset-0 size-full" />;
}

const DEFAULT_RATE: FrameRate = { num: 30, den: 1 };

/** The media item playing on its own (F1.17), with its name, length and frame rate. */
function useSourceMedia() {
  const id = useStore(playheadStore, (s) => s.source?.media ?? null);
  const media = useProjectStore((s) =>
    id === null ? undefined : s.state?.project.media.find((item) => item.id === id),
  );
  return id === null ? null : { id, media };
}

/**
 * F1.17: while a media item plays on its own, a bar above the picture names it and leads back to
 * the timeline (Esc). Above the picture, not over it: nothing in the DOM may cover the preview.
 */
function SourceBar() {
  const source = useSourceMedia();
  const ctx = useActionContext();
  if (!source) return null;
  const back = getAction("back-to-timeline");
  const name = source.media?.fileName ?? t.preview.source.unknown;
  return (
    <div
      className="flex h-8 shrink-0 items-center justify-between gap-2 border-b bg-accent px-3 text-sm"
      role="status"
    >
      <span className="truncate font-medium text-primary">{t.preview.source.previewing(name)}</span>
      <Button
        variant="ghost"
        size="sm"
        title={actionTooltip(back, ctx)}
        disabled={!isEnabled(back, ctx)}
        onClick={back.run}
      >
        <ArrowLeft className="size-3.5" aria-hidden />
        {actionLabel(back, ctx)}
      </Button>
    </div>
  );
}

/**
 * F1.59: playhead / timeline timecode (media time / length in a source preview), the Play button,
 * preview zoom and quality, the loop toggle (F1.67) and full screen. The buttons run the
 * registered actions, so they share labels, shortcuts and enabled state with the menu. Tooltips
 * open below the bar, never over the native preview above it; the menus hide the preview.
 */
function TransportBar() {
  const projectRate = useProjectStore((s) => s.state?.project.settings.frameRate ?? DEFAULT_RATE);
  const timelineLength = useProjectStore((s) => s.state?.project.duration ?? 0);
  const source = useSourceMedia();
  const ctx = useActionContext();
  const play = getAction("play-pause");
  const loop = getAction("loop");
  const fullScreen = getAction("full-screen");
  const rate = source ? (source.media?.frameRate ?? projectRate) : projectRate;
  const length = source ? (source.media?.duration ?? null) : timelineLength;

  return (
    <div className="grid h-11 shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-t px-3">
      <span className="flex items-center gap-1.5 font-mono text-sm tabular-nums">
        <PlayheadTimecode rate={rate} source={source?.id ?? null} />
        {length !== null && (
          <>
            <span className="text-muted-foreground" aria-hidden>
              {t.preview.timecodeOf}
            </span>
            <span className="text-muted-foreground" data-testid="timeline-length">
              {formatTimecode(length, rate)}
            </span>
          </>
        )}
      </span>
      <TransportButton
        label={actionLabel(play, ctx)}
        tooltip={actionTooltip(play, ctx)}
        disabled={!isEnabled(play, ctx)}
        onClick={play.run}
        variant="secondary"
      >
        {ctx.playing ? <Pause className="size-4" aria-hidden /> : <Play className="size-4" aria-hidden />}
      </TransportButton>
      <div className="flex items-center justify-end gap-1">
        <ZoomMenu />
        <QualityMenu />
        <TransportButton
          label={actionLabel(loop, ctx)}
          tooltip={actionTooltip(loop, ctx)}
          disabled={!isEnabled(loop, ctx)}
          onClick={loop.run}
          pressed={ctx.looping}
          variant="ghost"
        >
          <Repeat className="size-4" aria-hidden />
        </TransportButton>
        <TransportButton
          label={actionLabel(fullScreen, ctx)}
          tooltip={actionTooltip(fullScreen, ctx)}
          disabled={!isEnabled(fullScreen, ctx)}
          onClick={fullScreen.run}
          pressed={ctx.fullScreen}
          variant="ghost"
        >
          {ctx.fullScreen ? (
            <Minimize className="size-4" aria-hidden />
          ) : (
            <Maximize className="size-4" aria-hidden />
          )}
        </TransportButton>
      </div>
    </div>
  );
}

function TransportButton({
  label,
  tooltip,
  disabled,
  onClick,
  pressed,
  variant,
  children,
}: {
  label: string;
  tooltip: string;
  disabled: boolean;
  onClick: () => void;
  pressed?: boolean;
  variant: "secondary" | "ghost";
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant={variant}
          size="icon"
          aria-label={label}
          aria-pressed={pressed}
          className={cn(pressed && "bg-accent text-primary")}
          disabled={disabled}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom" avoidCollisions={false}>
        {tooltip}
      </TooltipContent>
    </Tooltip>
  );
}

function zoomText(zoom: PreviewZoom): string {
  return zoom.kind === "fit" ? t.preview.zoom.fit : t.preview.zoom.percent(zoom.percent);
}

/** A dropdown of the transport bar. It hides the native preview while open (ADR-0005). */
function BarMenu({ label, text, children }: { label: string; text: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  usePreviewOccluder(open);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" aria-label={label} title={label}>
          <span className="tabular-nums">{text}</span>
          <ChevronDown className="size-3.5" aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-44">
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** A menu entry that runs a registered action, with its shortcut. */
function ActionMenuItem({ id }: { id: ActionId }) {
  const ctx = useActionContext();
  const action = getAction(id);
  const shortcut = shortcutLabel(action);
  return (
    <DropdownMenuItem disabled={!isEnabled(action, ctx)} onSelect={action.run}>
      {actionLabel(action, ctx)}
      {shortcut && <DropdownMenuShortcut>{shortcut}</DropdownMenuShortcut>}
    </DropdownMenuItem>
  );
}

/** F1.62: Fit, the zoom levels, and the zoom steps. */
function ZoomMenu() {
  const zoom = usePreviewStore((s) => s.zoom);
  const text = zoomText(zoom);
  const isLevel = (percent: number) => zoom.kind === "percent" && zoom.percent === percent;
  return (
    <BarMenu label={t.preview.zoom.menu(text)} text={text}>
      <DropdownMenuCheckboxItem checked={zoom.kind === "fit"} onSelect={() => setPreviewZoom(FIT)}>
        {t.preview.zoom.fit}
        <DropdownMenuShortcut>{shortcutLabel(getAction("preview-zoom-fit"))}</DropdownMenuShortcut>
      </DropdownMenuCheckboxItem>
      {ZOOM_LEVELS.map((percent) => (
        <DropdownMenuCheckboxItem
          key={percent}
          checked={isLevel(percent)}
          onSelect={() => setPreviewZoom({ kind: "percent", percent })}
        >
          {t.preview.zoom.percent(percent)}
        </DropdownMenuCheckboxItem>
      ))}
      <DropdownMenuSeparator />
      <ActionMenuItem id="preview-zoom-in" />
      <ActionMenuItem id="preview-zoom-out" />
    </BarMenu>
  );
}

/** F1.61: Full, Half, Quarter; remembered on this computer. */
function QualityMenu() {
  const quality = usePreviewStore((s) => s.quality);
  const text = t.preview.quality.levels[quality];
  return (
    <BarMenu label={t.preview.quality.menu(text)} text={text}>
      {QUALITIES.map((level) => (
        <DropdownMenuCheckboxItem
          key={level}
          checked={quality === level}
          onSelect={() => setPreviewQuality(level)}
        >
          {t.preview.quality.levels[level]}
        </DropdownMenuCheckboxItem>
      ))}
      <DropdownMenuSeparator />
      <p className="max-w-56 px-2 py-1 text-xs text-muted-foreground">{t.preview.quality.hint}</p>
    </BarMenu>
  );
}

/**
 * The playhead timecode (media time in a source preview). Written straight into the DOM from a
 * store subscription, so playback ticks (up to 60 Hz) never re-render React; the text changes
 * once per frame at most.
 */
function PlayheadTimecode({ rate, source }: { rate: FrameRate; source: MediaId | null }) {
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const show = (s: ReturnType<typeof playheadStore.getState>) => {
      const time = source !== null && s.source?.media === source ? s.source.time : s.time;
      const text = formatTimecode(time, rate);
      if (element.textContent !== text) element.textContent = text;
    };
    show(playheadStore.getState());
    return playheadStore.subscribe(show);
  }, [rate, source]);

  return <span ref={ref} data-testid="playhead-timecode" />;
}
