import { FileWarning, FileX } from "lucide-react";
import { memo } from "react";

import type { MediaView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { formatDuration } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { MediaLayout } from "@/stores/uiStore";

import { KIND_ICONS, Thumbnail } from "./Thumbnail";

export type MediaItemProps = {
  item: MediaView_Serialize;
  layout: MediaLayout;
  selected: boolean;
  /** The item keyboard focus rests on (roving tabindex). */
  tabbable: boolean;
};

/** Kind, duration and size, e.g. "Video · 0:42 · 1080×1920". */
export function mediaDetails(item: MediaView_Serialize): string {
  const parts: string[] = [t.mediaBin.kinds[item.kind]];
  if (item.duration !== null) parts.push(formatDuration(item.duration));
  if (item.displayWidth !== null && item.displayHeight !== null) {
    parts.push(t.mediaBin.resolution(item.displayWidth, item.displayHeight));
  }
  return parts.join(t.mediaBin.detailSeparator);
}

function HdrBadge({ item }: { item: MediaView_Serialize }) {
  if (!item.hdr) return null;
  return (
    <span
      title={t.mediaBin.hdr.tip[item.hdr]}
      className={cn(
        "rounded-sm bg-background/80 px-1 text-2xs leading-4 font-semibold",
        item.hdr === "dolbyVisionOnly" ? "text-destructive" : "text-primary",
      )}
    >
      {t.mediaBin.hdr.badge[item.hdr]}
    </span>
  );
}

/** "Missing" or "Changed" for media that needs a relink (design §6.6). */
function StatusBadge({ item }: { item: MediaView_Serialize }) {
  if (item.status === "online") return null;
  const Icon = item.status === "offline" ? FileX : FileWarning;
  return (
    <span
      title={t.mediaBin.status.tip[item.status]}
      className={cn(
        "flex items-center gap-0.5 rounded-sm bg-background/80 px-1 text-2xs leading-4 font-semibold",
        item.status === "offline" ? "text-destructive" : "text-warning",
      )}
    >
      <Icon className="size-3" aria-hidden />
      {t.mediaBin.status.badge[item.status]}
    </span>
  );
}

/**
 * One media item, as a grid tile or a list row. Selection, keyboard and context menu are handled
 * by the parent list through `data-media-id` (event delegation), so the props stay plain data and
 * `memo` keeps unrelated items from re-rendering.
 */
export const MediaItem = memo(function MediaItem({ item, layout, selected, tabbable }: MediaItemProps) {
  const Icon = KIND_ICONS[item.kind];
  const duration = item.duration === null ? null : formatDuration(item.duration);
  const common = {
    role: "option",
    "aria-selected": selected,
    "data-media-id": item.id,
    tabIndex: tabbable ? 0 : -1,
    title: item.path,
  } as const;

  if (layout === "list") {
    return (
      <div
        {...common}
        className={cn(
          "flex items-center gap-2 rounded-sm px-2 py-1 outline-none select-none hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring",
          selected && "bg-accent hover:bg-accent",
        )}
      >
        <Thumbnail item={item} className="aspect-video w-14 shrink-0" />
        <div className="min-w-0 flex-1">
          <p className="truncate">{item.fileName}</p>
          <p className="flex items-center gap-1 truncate text-xs text-muted-foreground">
            <Icon className="size-3 shrink-0" aria-hidden />
            <span className="truncate">{mediaDetails(item)}</span>
          </p>
        </div>
        <StatusBadge item={item} />
        <HdrBadge item={item} />
      </div>
    );
  }

  return (
    <div
      {...common}
      className={cn(
        "flex min-w-0 flex-col gap-1 rounded-md p-1 outline-none select-none hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring",
        selected && "bg-accent ring-2 ring-primary hover:bg-accent",
      )}
    >
      <div className="relative">
        <Thumbnail item={item} className="aspect-video w-full" />
        <div className="pointer-events-none absolute inset-x-1 top-1 flex items-start justify-between">
          <span className="rounded-sm bg-background/80 p-0.5" title={t.mediaBin.kinds[item.kind]}>
            <Icon className="size-3 text-muted-foreground" aria-hidden />
            <span className="sr-only">{t.mediaBin.kinds[item.kind]}</span>
          </span>
          <span className="flex gap-1">
            <StatusBadge item={item} />
            <HdrBadge item={item} />
          </span>
        </div>
        {duration && (
          <span className="pointer-events-none absolute right-1 bottom-1 rounded-sm bg-background/80 px-1 text-2xs leading-4 tabular-nums">
            {duration}
          </span>
        )}
      </div>
      <p className="truncate px-0.5 text-xs">{item.fileName}</p>
    </div>
  );
});
