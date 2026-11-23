import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useStore } from "zustand";

import type { MediaView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { mediaDragStore } from "@/stores/mediaDragStore";

const OFFSET = 12;

/**
 * The label that follows the pointer while media is dragged. It re-renders only when a drag
 * starts or ends; the position is written to the DOM directly on every move.
 */
export function MediaDragChip({ media }: { media: readonly MediaView_Serialize[] }) {
  const chip = useRef<HTMLDivElement>(null);
  const first = useStore(mediaDragStore, (s) => s.drag?.media[0] ?? null);
  const count = useStore(mediaDragStore, (s) => s.drag?.media.length ?? 0);

  useEffect(
    () =>
      mediaDragStore.subscribe(({ drag }) => {
        if (drag && chip.current)
          chip.current.style.transform = `translate(${drag.x + OFFSET}px, ${drag.y + OFFSET}px)`;
      }),
    [],
  );

  if (first === null) return null;
  const start = mediaDragStore.getState().drag;
  const name = media.find((item) => item.id === first)?.fileName ?? "";
  return createPortal(
    <div
      ref={chip}
      aria-hidden
      className="pointer-events-none fixed top-0 left-0 z-50 max-w-60 truncate rounded-md border bg-popover px-2 py-1 text-xs text-popover-foreground shadow-md"
      style={start ? { transform: `translate(${start.x + OFFSET}px, ${start.y + OFFSET}px)` } : undefined}
    >
      {count > 1 ? t.mediaBin.add.dragMany(count) : name}
    </div>,
    document.body,
  );
}
