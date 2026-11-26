/**
 * The track header column: name, main-track badge, mute/hide/lock toggles (F1.52), drag to
 * reorder within the track's section (F1.53) and the track context menu (F1.56).
 */
import {
  ArrowDown,
  ArrowUp,
  AudioLines,
  Eye,
  EyeOff,
  Film,
  Lock,
  LockOpen,
  Trash2,
  Type,
  Volume2,
  VolumeX,
} from "lucide-react";
import { type ComponentType, type PointerEvent, useEffect, useEffectEvent, useRef, useState } from "react";

import type { TrackFlag, TrackKind, TrackView } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";

import { moveTrack, removeTrack, setTrackFlag } from "./editActions";
import { RULER_HEIGHT, TRACK_GAP, TRACK_HEIGHT } from "./geometry";
import { pastThreshold } from "./hitTest";
import { boundaryY, moveRange, type TrackDrop, trackDropAt, trackFlags } from "./tracks";

type Icon = ComponentType<{ className?: string }>;

const KIND_ICONS: Record<TrackKind, Icon> = { video: Film, audio: AudioLines, text: Type };

/** Icon when the flag is on / off. */
const FLAG_ICONS: Record<TrackFlag, { on: Icon; off: Icon }> = {
  muted: { on: VolumeX, off: Volume2 },
  hidden: { on: EyeOff, off: Eye },
  locked: { on: Lock, off: LockOpen },
};

const flagValue = (track: TrackView, flag: TrackFlag) => track[flag];

type Press = { from: number; y0: number; pointerId: number; dragging: boolean };

export function TrackHeaders({
  tracks,
  mainTrack,
  height,
}: {
  tracks: TrackView[];
  mainTrack: string | null;
  height: number;
}) {
  const column = useRef<HTMLDivElement>(null);
  const press = useRef<Press | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [drop, setDrop] = useState<TrackDrop | null>(null);
  // One flag for every header's menu: only one context menu is open at a time.
  const [menuOpen, setMenuOpen] = useState(false);
  usePreviewOccluder(menuOpen);

  const localY = (clientY: number) => clientY - (column.current?.getBoundingClientRect().top ?? 0);

  const reset = () => {
    const current = press.current;
    press.current = null;
    setDragging(null);
    setDrop(null);
    if (current && column.current?.hasPointerCapture(current.pointerId))
      column.current.releasePointerCapture(current.pointerId);
  };

  // Escape and window blur cancel a header drag; Escape must not also deselect clips.
  const cancel = useEffectEvent(reset);
  const active = dragging !== null;
  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      cancel();
    };
    const onBlur = () => cancel();
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [active]);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !(event.target instanceof Element) || event.target.closest("button")) return;
    const header = event.target.closest<HTMLElement>("[data-track-index]");
    const from = Number(header?.dataset.trackIndex);
    if (!header || !moveRange(tracks, mainTrack, from)) return;
    press.current = { from, y0: event.clientY, pointerId: event.pointerId, dragging: false };
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || p.pointerId !== event.pointerId) return;
    if (!p.dragging) {
      if ((event.buttons & 1) === 0 || !pastThreshold(0, event.clientY - p.y0)) return;
      p.dragging = true;
      event.currentTarget.setPointerCapture(event.pointerId);
      setDragging(p.from);
    }
    setDrop(trackDropAt(tracks, mainTrack, p.from, localY(event.clientY)));
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || p.pointerId !== event.pointerId) return;
    const target = p.dragging ? trackDropAt(tracks, mainTrack, p.from, localY(event.clientY)) : null;
    const track = tracks[p.from];
    reset();
    if (target && track) void moveTrack(track.id, target.index);
  };

  return (
    <div
      ref={column}
      className="relative w-56 shrink-0 border-r bg-panel select-none"
      style={{ minHeight: height }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={reset}
      onLostPointerCapture={() => {
        if (press.current?.dragging) reset();
      }}
    >
      <div style={{ height: RULER_HEIGHT }} className="border-b" />
      {tracks.map((track, index) => (
        <TrackHeader
          key={track.id}
          track={track}
          index={index}
          tracks={tracks}
          mainTrack={mainTrack}
          dragging={dragging === index}
          onMenuOpenChange={setMenuOpen}
        />
      ))}
      {drop && (
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 h-0.5 -translate-y-1/2 bg-primary"
          style={{ top: boundaryY(drop.boundary) }}
        />
      )}
    </div>
  );
}

function TrackHeader({
  track,
  index,
  tracks,
  mainTrack,
  dragging,
  onMenuOpenChange,
}: {
  track: TrackView;
  index: number;
  tracks: TrackView[];
  mainTrack: string | null;
  dragging: boolean;
  onMenuOpenChange: (open: boolean) => void;
}) {
  const Icon = KIND_ICONS[track.kind];
  const main = track.id === mainTrack;
  const range = moveRange(tracks, mainTrack, index);
  const removeLabel = t.timeline.removeTrack(track.name);
  const flags = trackFlags(track.kind);
  return (
    <ContextMenu onOpenChange={onMenuOpenChange}>
      <ContextMenuTrigger asChild>
        <div
          data-track-index={index}
          className={cn(
            "group flex items-center gap-1.5 border-b border-l-2 border-transparent pr-1 pl-2",
            main && "border-l-primary bg-accent/40",
            range && "cursor-grab",
            dragging && "cursor-grabbing bg-accent",
          )}
          style={{ height: TRACK_HEIGHT, marginBottom: TRACK_GAP }}
          title={range ? t.timeline.reorderTip : undefined}
        >
          <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <span
            className={cn("min-w-0 flex-1 truncate font-medium", track.hidden && "text-muted-foreground")}
          >
            {track.name}
          </span>
          {main && (
            <span
              className="rounded-sm bg-primary/20 px-1 text-2xs leading-4 font-semibold text-primary"
              title={t.timeline.mainTrackTip}
            >
              {t.timeline.mainTrack}
            </span>
          )}
          {flags.map((flag) => (
            <FlagButton key={flag} track={track} flag={flag} />
          ))}
          {/* The main track cannot be removed (timeline design §3.3). */}
          {!main && (
            <Button
              variant="ghost"
              size="icon-xs"
              className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
              aria-label={removeLabel}
              title={removeLabel}
              onClick={() => void removeTrack(track.id)}
            >
              <Trash2 className="size-3.5" aria-hidden />
            </Button>
          )}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-48">
        {flags.map((flag) => (
          <ContextMenuCheckboxItem
            key={flag}
            checked={flagValue(track, flag)}
            onSelect={() => void setTrackFlag(track.id, flag, !flagValue(track, flag))}
          >
            {t.timeline.flags[flag].menu}
          </ContextMenuCheckboxItem>
        ))}
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={!range || index <= range.min}
          onSelect={() => void moveTrack(track.id, index - 1)}
        >
          <ArrowUp aria-hidden />
          {t.timeline.moveTrackUp}
        </ContextMenuItem>
        <ContextMenuItem
          disabled={!range || index >= range.max}
          onSelect={() => void moveTrack(track.id, index + 1)}
        >
          <ArrowDown aria-hidden />
          {t.timeline.moveTrackDown}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" disabled={main} onSelect={() => void removeTrack(track.id)}>
          <Trash2 aria-hidden />
          {t.timeline.deleteTrack}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** One header toggle; pressed while the flag is on. */
function FlagButton({ track, flag }: { track: TrackView; flag: TrackFlag }) {
  const on = flagValue(track, flag);
  const { on: OnIcon, off: OffIcon } = FLAG_ICONS[flag];
  const FlagIcon = on ? OnIcon : OffIcon;
  const text = t.timeline.flags[flag];
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      aria-label={text.label(track.name)}
      aria-pressed={on}
      title={on ? text.on : text.off}
      className={cn(
        "text-muted-foreground",
        on && "bg-accent text-primary",
        // Off toggles stay out of the way until the header is hovered or focused.
        !on && "opacity-60 group-hover:opacity-100 focus-visible:opacity-100",
      )}
      onClick={() => void setTrackFlag(track.id, flag, !on)}
    >
      <FlagIcon className="size-3.5" aria-hidden />
    </Button>
  );
}
