import { Pause, Play, Repeat } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";

import { commands, type FrameRate } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  actionLabel,
  actionTooltip,
  getAction,
  isEnabled,
  useActionContext,
} from "@/features/editor/actions";
import { t } from "@/i18n";
import { mediaUrl } from "@/ipc/media";
import { formatTimecode } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";

/** Frames mode only: upper bound on how often the UI pulls a frame. */
const FRAME_INTERVAL_MS = 1000 / 30;

export function PreviewArea() {
  const mode = useAppStore((s) => s.info?.previewMode);

  return (
    <div className="flex h-full flex-col bg-panel">
      <div className="relative min-h-0 flex-1 bg-preview-bg">
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
      void commands.setPreviewRect({
        x: Math.round(box.left * dpr),
        y: Math.round(box.top * dpr),
        width: Math.max(0, Math.round(box.width * dpr)),
        height: Math.max(0, Math.round(box.height * dpr)),
      });
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    // Panel resizes move the slot without changing its size; window resizes change DPR or layout.
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

/**
 * F1.59: playhead / timeline timecode, the Play button and the loop toggle (F1.67). The buttons
 * run the registered actions, so they share labels, shortcuts and enabled state with the menu.
 * Tooltips open below the bar, never over the native preview above it.
 */
function TransportBar() {
  const rate = useProjectStore((s) => s.state?.project.settings.frameRate ?? DEFAULT_RATE);
  const duration = useProjectStore((s) => s.state?.project.duration ?? 0);
  const ctx = useActionContext();
  const play = getAction("play-pause");
  const loop = getAction("loop");

  return (
    <div className="grid h-11 shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-t px-3">
      <span className="flex items-center gap-1.5 font-mono text-sm tabular-nums">
        <PlayheadTimecode rate={rate} />
        <span className="text-muted-foreground" aria-hidden>
          {t.preview.timecodeOf}
        </span>
        <span className="text-muted-foreground" data-testid="timeline-length">
          {formatTimecode(duration, rate)}
        </span>
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
      <div className="flex justify-end">
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

/**
 * The playhead timecode. Written straight into the DOM from a store subscription, so playback
 * ticks (up to 60 Hz) never re-render React; the text changes once per frame at most.
 */
function PlayheadTimecode({ rate }: { rate: FrameRate }) {
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const show = (time: number) => {
      const text = formatTimecode(time, rate);
      if (element.textContent !== text) element.textContent = text;
    };
    show(playheadStore.getState().time);
    return playheadStore.subscribe((s) => show(s.time));
  }, [rate]);

  return <span ref={ref} data-testid="playhead-timecode" />;
}
