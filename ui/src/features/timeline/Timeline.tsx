import {
  AudioLines,
  Film,
  FoldHorizontal,
  Link2,
  Magnet,
  Maximize2,
  Plus,
  Scissors,
  Trash2,
  Type,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { type ComponentType, useEffect, useMemo, useRef } from "react";
import { useStore } from "zustand";

import {
  commands,
  type MediaId,
  type MediaView_Serialize,
  type TrackId,
  type TrackKind,
  type TrackView,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  type ActionId,
  actionLabel,
  actionTooltip,
  getAction,
  isEnabled,
  useActionContext,
} from "@/features/editor/actions";
import { t } from "@/i18n";
import { importPathsForPlacement } from "@/features/media-bin/importActions";
import { useFileDrop } from "@/features/media-bin/useFileDrop";
import { formatClipDuration, frameDuration } from "@/lib/time";
import { cn } from "@/lib/utils";
import { run } from "@/stores/appStore";
import { mediaDragStore, registerDropZone } from "@/stores/mediaDragStore";
import { playheadStore } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { selectionStore } from "@/stores/selectionStore";
import { setLaneWidth, setViewport, timelineViewStore } from "@/stores/timelineViewStore";
import { useUiStore } from "@/stores/uiStore";

import {
  DEFAULT_VIEWPORT,
  TRACK_GAP,
  ZOOM_SLIDER_STEPS,
  scrollBy,
  sliderToZoom,
  trackTop,
  zoomAt,
  zoomToSlider,
  type Viewport,
} from "./geometry";
import { addMediaClips } from "./addActions";
import { ClipContextMenu } from "./ClipContextMenu";
import { type Hit, hitTest } from "./hitTest";
import { TrackHeaders } from "./TrackHeaders";
import { locateGap } from "./gap";
import { dropPayload, dropPlacement, dropPreview, type DropPreview } from "./drop";
import { createClipMedia } from "./clipMedia";
import { setZoomLevel } from "./navActions";
import { attachPointerInteraction, type PointerOverlay } from "./pointer";
import { drawTimeline, readTheme, type TimelineTheme } from "./renderer";

const EMPTY_TRACKS: TrackView[] = [];
const EMPTY_MEDIA: MediaView_Serialize[] = [];

export function Timeline() {
  const tracks = useProjectStore((s) => s.state?.project.tracks ?? EMPTY_TRACKS);
  const mainTrack = useProjectStore((s) => s.state?.project.mainTrack ?? null);
  const media = useProjectStore((s) => s.state?.project.media ?? EMPTY_MEDIA);
  const offline = useMemo(
    () => new Set(media.filter((item) => item.status === "offline").map((item) => item.id)),
    [media],
  );
  const mediaById = useMemo(() => new Map(media.map((item) => [item.id, item])), [media]);
  const contentHeight = trackTop(tracks.length) + TRACK_GAP;

  return (
    <div className="flex h-full flex-col bg-timeline">
      <TimelineToolbar />
      <div className="flex min-h-0 flex-1 overflow-y-auto">
        <TrackHeaders tracks={tracks} mainTrack={mainTrack} height={contentHeight} />
        <TimelineCanvas
          tracks={tracks}
          mainTrack={mainTrack}
          offline={offline}
          media={mediaById}
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
      <div className="mx-1 h-5 w-px bg-border" aria-hidden />
      <ActionButton id="split" icon={Scissors} />
      <ActionButton id="delete" icon={Trash2} />
      <ActionButton id="link-toggle" icon={Link2} />
      <div className="mx-1 h-5 w-px bg-border" aria-hidden />
      <ActionButton id="magnetic" icon={FoldHorizontal} />
      <SnappingToggle />
      <div className="ml-auto flex items-center gap-1">
        <ActionButton id="zoom-out" icon={ZoomOut} />
        <ZoomSlider />
        <ActionButton id="zoom-in" icon={ZoomIn} />
        <ActionButton id="zoom-to-fit" icon={Maximize2} />
      </div>
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

/**
 * A toolbar button for a registered action: the same label, shortcut, enabled and checked state
 * as its menu entry.
 */
function ActionButton({ id, icon: Icon }: { id: ActionId; icon: ComponentType<{ className?: string }> }) {
  const ctx = useActionContext();
  const action = getAction(id);
  const checked = action.checked?.(ctx);
  const label = actionTooltip(action, ctx);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-label={actionLabel(action, ctx)}
          aria-pressed={checked}
          className={cn(checked && "bg-accent text-primary")}
          disabled={!isEnabled(action, ctx)}
          onClick={action.run}
        >
          <Icon className="size-3.5" aria-hidden />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/** F1.45: turns snapping on or off; a UI preference kept across restarts. */
function SnappingToggle() {
  const snapping = useUiStore((s) => s.snapping);
  const setSnapping = useUiStore((s) => s.setSnapping);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-pressed={snapping}
          aria-label={t.timeline.snapping}
          className={cn(snapping && "bg-accent text-primary")}
          onClick={() => setSnapping(!snapping)}
        >
          <Magnet className="size-3.5" aria-hidden />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{snapping ? t.timeline.snappingOn : t.timeline.snappingOff}</TooltipContent>
    </Tooltip>
  );
}

/**
 * F1.32: the zoom level on a logarithmic slider, anchored at the playhead (or the middle). It
 * re-renders on zoom changes only, never on scroll or playhead moves.
 */
function ZoomSlider() {
  const flicksPerPixel = useStore(timelineViewStore, (s) => s.viewport.flicksPerPixel);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className="w-28 px-1">
          <Slider
            aria-label={t.timeline.zoom}
            min={0}
            max={ZOOM_SLIDER_STEPS}
            step={1}
            value={[zoomToSlider(flicksPerPixel)]}
            onValueChange={([position]) => {
              if (position !== undefined) setZoomLevel(sliderToZoom(position));
            }}
          />
        </div>
      </TooltipTrigger>
      <TooltipContent>{t.timeline.zoomHint}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The canvas part. Viewport and playhead live in vanilla stores and trigger a redraw
 * directly, so scrolling, zooming and scrubbing never re-render React.
 */
function TimelineCanvas({
  tracks,
  mainTrack,
  offline,
  media,
  contentHeight,
}: {
  tracks: TrackView[];
  mainTrack: TrackId | null;
  offline: ReadonlySet<MediaId>;
  media: ReadonlyMap<MediaId, MediaView_Serialize>;
  contentHeight: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const sceneRef = useRef({ tracks, mainTrack, offline, media });
  const dropRef = useRef<DropPreview | null>(null);
  const overlayRef = useRef<PointerOverlay>({ marquee: null, snapLine: null });
  const requestDrawRef = useRef<() => void>(() => undefined);

  useEffect(() => {
    sceneRef.current = { tracks, mainTrack, offline, media };
    requestDrawRef.current();
  }, [tracks, mainTrack, offline, media]);

  // Files from Explorer: one import step, then one add step at the drop position (F1.03).
  const hoveringFiles = useFileDrop(containerRef, (paths, point) => {
    void (async () => {
      const media = await importPathsForPlacement(paths);
      const canvas = canvasRef.current;
      if (media.length === 0 || !canvas) return;
      const preview = previewAt(
        canvas,
        timelineViewStore.getState().viewport,
        media,
        point.x,
        point.y,
        false,
      );
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
    // A fresh timeline starts at the default zoom, scrolled to 0, as before the view was shared.
    setViewport(DEFAULT_VIEWPORT);
    const viewport = () => timelineViewStore.getState().viewport;
    const requestDraw = () => {
      frame ||= requestAnimationFrame(draw);
    };
    // Filmstrip tiles and waveforms; a finished load redraws.
    const products = createClipMedia(requestDraw);

    const draw = () => {
      frame = 0;
      const dpr = window.devicePixelRatio;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      products.beginFrame();
      drawTimeline(ctx, {
        ...size,
        ...sceneRef.current,
        products,
        viewport: viewport(),
        playhead: playheadStore.getState().time,
        theme,
        offlineLabel: t.timeline.offline,
        drop: dropRef.current,
        selected: new Set(selectionStore.getState().clips),
        gap: locateGap(sceneRef.current.tracks, selectionStore.getState().gap),
        marquee: overlayRef.current.marquee,
        snapLine: overlayRef.current.snapLine,
        durationText: formatClipDuration,
      });
      products.endFrame();
    };
    requestDrawRef.current = requestDraw;

    const resize = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const dpr = window.devicePixelRatio;
      size = { width: entry.contentRect.width, height: entry.contentRect.height };
      canvas.width = Math.round(size.width * dpr);
      canvas.height = Math.round(size.height * dpr);
      setLaneWidth(size.width);
      theme = readTheme(container);
      requestDraw();
    });
    resize.observe(container);

    // Only the time is drawn; a new seek generation alone needs no redraw.
    const unsubscribePlayhead = playheadStore.subscribe((s, prev) => {
      if (s.time !== prev.time) requestDraw();
    });
    const unsubscribeSelection = selectionStore.subscribe(requestDraw);
    const unsubscribeView = timelineViewStore.subscribe(requestDraw);

    // Media dragged from the media panel: follow the pointer with a drop preview.
    const unsubscribeDrag = mediaDragStore.subscribe(({ drag }) => {
      const next = drag ? previewAt(canvas, viewport(), drag.media, drag.x, drag.y, drag.alt) : null;
      if (next === null && dropRef.current === null) return;
      dropRef.current = next;
      requestDraw();
    });
    const unregisterDrop = registerDropZone((drag) => {
      const preview = previewAt(canvas, viewport(), drag.media, drag.x, drag.y, drag.alt);
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
        setViewport(zoomAt(viewport(), Math.exp(event.deltaY * 0.002), x));
      } else if (event.deltaX !== 0 || event.shiftKey) {
        event.preventDefault();
        setViewport(scrollBy(viewport(), event.deltaX || event.deltaY));
      }
      // Plain vertical wheel scrolls the track list.
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });

    const detachPointer = attachPointerInteraction(canvas, {
      viewport,
      overlay: overlayRef.current,
      requestDraw,
    });

    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      unsubscribePlayhead();
      unsubscribeSelection();
      unsubscribeView();
      products.clear();
      detachPointer();
      unsubscribeDrag();
      unregisterDrop();
      canvas.removeEventListener("wheel", onWheel);
      requestDrawRef.current = () => undefined;
    };
  }, []);

  const hitAt = (clientX: number, clientY: number): Hit | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const tracks = useProjectStore.getState().state?.project.tracks ?? [];
    return hitTest(clientX - rect.left, clientY - rect.top, tracks, timelineViewStore.getState().viewport);
  };

  return (
    <ClipContextMenu hitAt={hitAt}>
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
    </ClipContextMenu>
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
    snapping: useUiStore.getState().snapping && !alt,
  });
}
