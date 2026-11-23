import { AudioLines, Film, Plus, Trash2, Type } from "lucide-react";
import { useEffect, useRef, type ComponentType } from "react";

import { commands, type TrackKind, type TrackView } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { run } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";

import {
  DEFAULT_VIEWPORT,
  RULER_HEIGHT,
  TRACK_GAP,
  TRACK_HEIGHT,
  scrollBy,
  trackTop,
  xToTime,
  zoomAt,
  type Viewport,
} from "./geometry";
import { startPlayheadDrag } from "./playheadDrag";
import { drawTimeline, readTheme, type TimelineTheme } from "./renderer";

const EMPTY_TRACKS: TrackView[] = [];

const KIND_ICONS: Record<TrackKind, ComponentType<{ className?: string }>> = {
  video: Film,
  audio: AudioLines,
  text: Type,
};

export function Timeline() {
  const tracks = useProjectStore((s) => s.state?.project.tracks ?? EMPTY_TRACKS);
  const contentHeight = trackTop(tracks.length) + TRACK_GAP;

  return (
    <div className="flex h-full flex-col bg-timeline">
      <TimelineToolbar />
      <div className="flex min-h-0 flex-1 overflow-y-auto">
        <div className="w-52 shrink-0 border-r bg-panel" style={{ minHeight: contentHeight }}>
          <div style={{ height: RULER_HEIGHT }} className="border-b" />
          {tracks.map((track) => (
            <TrackHeader key={track.id} track={track} />
          ))}
        </div>
        <TimelineCanvas tracks={tracks} contentHeight={contentHeight} />
      </div>
    </div>
  );
}

function TimelineToolbar() {
  const addTrack = (kind: TrackKind) => void run(commands.addTrack(kind, null));
  return (
    <div className="flex h-9 shrink-0 items-center gap-1 border-b px-2">
      <AddTrackButton label={t.timeline.addVideoTrack} icon={Film} onClick={() => addTrack("video")} />
      <AddTrackButton label={t.timeline.addAudioTrack} icon={AudioLines} onClick={() => addTrack("audio")} />
      <AddTrackButton label={t.timeline.addTextTrack} icon={Type} onClick={() => addTrack("text")} />
      <span className="ml-auto text-xs text-muted-foreground">{t.timeline.zoomHint}</span>
    </div>
  );
}

function AddTrackButton({
  label,
  icon: Icon,
  onClick,
}: {
  label: string;
  icon: ComponentType<{ className?: string }>;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="sm" onClick={onClick} aria-label={label}>
          <Plus className="size-3.5" aria-hidden />
          <Icon className="size-3.5" aria-hidden />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function TrackHeader({ track }: { track: TrackView }) {
  const Icon = KIND_ICONS[track.kind];
  const removeLabel = t.timeline.removeTrack(track.name);
  return (
    <div
      className="group flex items-center gap-2 border-b border-transparent px-3"
      style={{ height: TRACK_HEIGHT, marginBottom: TRACK_GAP }}
    >
      <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <span className="min-w-0 flex-1 truncate font-medium">{track.name}</span>
      <Button
        variant="ghost"
        size="icon"
        className="size-7 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
        aria-label={removeLabel}
        title={removeLabel}
        onClick={() => void run(commands.removeTrack(track.id))}
      >
        <Trash2 className="size-3.5" aria-hidden />
      </Button>
    </div>
  );
}

/**
 * The canvas part. Viewport and playhead live in refs/vanilla stores and trigger a redraw
 * directly, so scrolling, zooming and scrubbing never re-render React.
 */
function TimelineCanvas({ tracks, contentHeight }: { tracks: TrackView[]; contentHeight: number }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewportRef = useRef<Viewport>(DEFAULT_VIEWPORT);
  const tracksRef = useRef(tracks);
  const requestDrawRef = useRef<() => void>(() => undefined);

  useEffect(() => {
    tracksRef.current = tracks;
    requestDrawRef.current();
  }, [tracks]);

  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!container || !canvas || !ctx) return;

    let theme: TimelineTheme = readTheme(container);
    let size = { width: 0, height: 0 };
    let frame = 0;

    const draw = () => {
      frame = 0;
      const dpr = window.devicePixelRatio;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawTimeline(ctx, {
        ...size,
        tracks: tracksRef.current,
        viewport: viewportRef.current,
        playhead: playheadStore.getState().time,
        theme,
      });
    };
    const requestDraw = () => {
      frame ||= requestAnimationFrame(draw);
    };
    requestDrawRef.current = requestDraw;

    const resize = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const dpr = window.devicePixelRatio;
      size = { width: entry.contentRect.width, height: entry.contentRect.height };
      canvas.width = Math.round(size.width * dpr);
      canvas.height = Math.round(size.height * dpr);
      theme = readTheme(container);
      requestDraw();
    });
    resize.observe(container);

    // Only the time is drawn; a new seek generation alone needs no redraw.
    const unsubscribePlayhead = playheadStore.subscribe((s, prev) => {
      if (s.time !== prev.time) requestDraw();
    });

    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey) {
        event.preventDefault();
        const x = event.clientX - canvas.getBoundingClientRect().left;
        viewportRef.current = zoomAt(viewportRef.current, Math.exp(event.deltaY * 0.002), x);
      } else if (event.deltaX !== 0 || event.shiftKey) {
        event.preventDefault();
        viewportRef.current = scrollBy(viewportRef.current, event.deltaX || event.deltaY);
      } else {
        return; // Plain vertical wheel scrolls the track list.
      }
      requestDraw();
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });

    const timeAt = (event: PointerEvent) =>
      xToTime(event.clientX - canvas.getBoundingClientRect().left, viewportRef.current);
    const onPointerDown = (event: PointerEvent) => {
      if (event.button === 0) startPlayheadDrag(canvas, event, timeAt);
    };
    canvas.addEventListener("pointerdown", onPointerDown);

    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      unsubscribePlayhead();
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("pointerdown", onPointerDown);
      requestDrawRef.current = () => undefined;
    };
  }, []);

  return (
    <div ref={containerRef} className="relative min-w-0 flex-1" style={{ minHeight: contentHeight }}>
      <canvas ref={canvasRef} className="absolute inset-0 size-full" />
    </div>
  );
}
