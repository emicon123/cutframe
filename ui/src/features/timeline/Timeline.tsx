import { AudioLines, Film, Lock, Plus, Trash2, Type } from "lucide-react";
import { type ComponentType, useEffect, useMemo, useRef } from "react";

import {
  commands,
  type MediaId,
  type MediaView_Serialize,
  type TrackId,
  type TrackKind,
  type TrackView,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { importPathsForPlacement } from "@/features/media-bin/importActions";
import { useFileDrop } from "@/features/media-bin/useFileDrop";
import { frameDuration } from "@/lib/time";
import { cn } from "@/lib/utils";
import { run } from "@/stores/appStore";
import { mediaDragStore, registerDropZone } from "@/stores/mediaDragStore";
import { playheadStore, setPlayhead } from "@/stores/playheadStore";
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
import { addMediaClips } from "./addActions";
import { dropPayload, dropPlacement, dropPreview, type DropPreview } from "./drop";
import { drawTimeline, readTheme, type TimelineTheme } from "./renderer";

const EMPTY_TRACKS: TrackView[] = [];
const EMPTY_MEDIA: MediaView_Serialize[] = [];

const KIND_ICONS: Record<TrackKind, ComponentType<{ className?: string }>> = {
  video: Film,
  audio: AudioLines,
  text: Type,
};

export function Timeline() {
  const tracks = useProjectStore((s) => s.state?.project.tracks ?? EMPTY_TRACKS);
  const mainTrack = useProjectStore((s) => s.state?.project.mainTrack ?? null);
  const media = useProjectStore((s) => s.state?.project.media ?? EMPTY_MEDIA);
  const offline = useMemo(
    () => new Set(media.filter((item) => item.status === "offline").map((item) => item.id)),
    [media],
  );
  const contentHeight = trackTop(tracks.length) + TRACK_GAP;

  return (
    <div className="flex h-full flex-col bg-timeline">
      <TimelineToolbar />
      <div className="flex min-h-0 flex-1 overflow-y-auto">
        <div className="w-52 shrink-0 border-r bg-panel" style={{ minHeight: contentHeight }}>
          <div style={{ height: RULER_HEIGHT }} className="border-b" />
          {tracks.map((track) => (
            <TrackHeader key={track.id} track={track} main={track.id === mainTrack} />
          ))}
        </div>
        <TimelineCanvas
          tracks={tracks}
          mainTrack={mainTrack}
          offline={offline}
          contentHeight={contentHeight}
        />
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

function TrackHeader({ track, main }: { track: TrackView; main: boolean }) {
  const Icon = KIND_ICONS[track.kind];
  const removeLabel = t.timeline.removeTrack(track.name);
  return (
    <div
      className={cn(
        "group flex items-center gap-2 border-b border-l-2 border-transparent px-3",
        main && "border-l-primary bg-accent/40",
      )}
      style={{ height: TRACK_HEIGHT, marginBottom: TRACK_GAP }}
    >
      <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <span className="min-w-0 flex-1 truncate font-medium">{track.name}</span>
      {main && (
        <span
          className="rounded-sm bg-primary/20 px-1 text-2xs leading-4 font-semibold text-primary"
          title={t.timeline.mainTrackTip}
        >
          {t.timeline.mainTrack}
        </span>
      )}
      {track.locked && (
        <span title={t.timeline.locked}>
          <Lock className="size-3.5 text-muted-foreground" aria-hidden />
          <span className="sr-only">{t.timeline.locked}</span>
        </span>
      )}
      {/* The main track cannot be removed (timeline design §3.3). */}
      {!main && (
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
      )}
    </div>
  );
}

/**
 * The canvas part. Viewport and playhead live in refs/vanilla stores and trigger a redraw
 * directly, so scrolling, zooming and scrubbing never re-render React.
 */
function TimelineCanvas({
  tracks,
  mainTrack,
  offline,
  contentHeight,
}: {
  tracks: TrackView[];
  mainTrack: TrackId | null;
  offline: ReadonlySet<MediaId>;
  contentHeight: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewportRef = useRef<Viewport>(DEFAULT_VIEWPORT);
  const sceneRef = useRef({ tracks, mainTrack, offline });
  const dropRef = useRef<DropPreview | null>(null);
  const requestDrawRef = useRef<() => void>(() => undefined);

  useEffect(() => {
    sceneRef.current = { tracks, mainTrack, offline };
    requestDrawRef.current();
  }, [tracks, mainTrack, offline]);

  // Files from Explorer: one import step, then one add step at the drop position (F1.03).
  const hoveringFiles = useFileDrop(containerRef, (paths, point) => {
    void (async () => {
      const media = await importPathsForPlacement(paths);
      const canvas = canvasRef.current;
      if (media.length === 0 || !canvas) return;
      const preview = previewAt(canvas, viewportRef.current, media, point.x, point.y, false);
      const placement = (preview && dropPlacement(preview)) ?? { kind: "append" };
      await addMediaClips(media, placement);
    })();
  });

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
        ...sceneRef.current,
        viewport: viewportRef.current,
        playhead: playheadStore.getState().time,
        theme,
        offlineLabel: t.timeline.offline,
        drop: dropRef.current,
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

    const unsubscribePlayhead = playheadStore.subscribe(requestDraw);

    // Media dragged from the media panel: follow the pointer with a drop preview.
    const unsubscribeDrag = mediaDragStore.subscribe(({ drag }) => {
      const next = drag ? previewAt(canvas, viewportRef.current, drag.media, drag.x, drag.y, drag.alt) : null;
      if (next === null && dropRef.current === null) return;
      dropRef.current = next;
      requestDraw();
    });
    const unregisterDrop = registerDropZone((drag) => {
      const preview = previewAt(canvas, viewportRef.current, drag.media, drag.x, drag.y, drag.alt);
      dropRef.current = null;
      requestDraw();
      if (!preview) return false;
      const placement = dropPlacement(preview);
      if (placement) void addMediaClips(drag.media, placement);
      return true;
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

    const scrubTo = (event: PointerEvent) => {
      const x = event.clientX - canvas.getBoundingClientRect().left;
      setPlayhead(xToTime(x, viewportRef.current));
    };
    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0) return;
      canvas.setPointerCapture(event.pointerId);
      scrubTo(event);
    };
    const onPointerMove = (event: PointerEvent) => {
      if (canvas.hasPointerCapture(event.pointerId)) scrubTo(event);
    };
    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointermove", onPointerMove);

    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      unsubscribePlayhead();
      unsubscribeDrag();
      unregisterDrop();
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      requestDrawRef.current = () => undefined;
    };
  }, []);

  return (
    <div
      ref={containerRef}
      className={cn("relative min-w-0 flex-1", hoveringFiles && "ring-2 ring-primary ring-inset")}
      style={{ minHeight: contentHeight }}
    >
      <canvas ref={canvasRef} className="absolute inset-0 size-full" />
      {hoveringFiles && (
        <div className="pointer-events-none absolute inset-x-0 top-8 flex justify-center">
          <span className="rounded-md bg-popover px-2 py-1 text-xs font-medium text-primary shadow-md">
            {t.timeline.dropFiles}
          </span>
        </div>
      )}
    </div>
  );
}

/**
 * The drop preview for `media` with the pointer at client (`clientX`, `clientY`), from the current
 * project; `null` outside the lanes.
 */
function previewAt(
  canvas: HTMLCanvasElement,
  viewport: Viewport,
  media: readonly MediaId[],
  clientX: number,
  clientY: number,
  alt: boolean,
): DropPreview | null {
  const rect = canvas.getBoundingClientRect();
  const x = clientX - rect.left;
  const y = clientY - rect.top;
  if (x < 0 || y < 0 || x >= rect.width || y >= rect.height) return null;
  const project = useProjectStore.getState().state?.project;
  if (!project) return null;
  const items = media.flatMap((id) => project.media.find((item) => item.id === id) ?? []);
  if (items.length === 0) return null;
  return dropPreview({
    x,
    y,
    tracks: project.tracks,
    mainTrack: project.mainTrack,
    magnetic: project.magnetic,
    viewport,
    playhead: playheadStore.getState().time,
    frame: frameDuration(project.settings.frameRate),
    payload: dropPayload(items),
    snapping: !alt,
  });
}
