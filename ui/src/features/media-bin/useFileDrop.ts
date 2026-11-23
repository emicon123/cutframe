import { getCurrentWebview } from "@tauri-apps/api/webview";
import { type RefObject, useEffect, useEffectEvent, useState } from "react";

import { logError } from "@/ipc/errors";

import { containsPoint, isLikelyImportable, type Point, toCssPoint } from "./dropTarget";

/**
 * Receives files and folders dragged from Explorer onto `target` through Tauri's webview
 * drag-drop event (HTML5 `drop` carries no paths). Returns whether a drag is over the target, for
 * the highlight. Internal drags must use pointer events: with native file drop on, WebView2 does
 * not deliver HTML5 drag-and-drop events (design media-import §2).
 */
export function useFileDrop(
  target: RefObject<HTMLElement | null>,
  /** `point` is the drop position in CSS client pixels. */
  onDrop: (paths: string[], point: Point) => void,
): boolean {
  const [hovering, setHovering] = useState(false);
  const drop = useEffectEvent(onDrop);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let dragged: string[] = [];

    const isOver = (physical: Point): boolean => {
      const element = target.current;
      if (!element) return false;
      return containsPoint(element.getBoundingClientRect(), toCssPoint(physical, window.devicePixelRatio));
    };

    // `getCurrentWebview()` throws outside Tauri; keep that inside the promise chain.
    Promise.resolve()
      .then(() =>
        getCurrentWebview().onDragDropEvent(({ payload }) => {
          switch (payload.type) {
            case "enter":
              dragged = payload.paths;
              setHovering(isLikelyImportable(dragged) && isOver(payload.position));
              break;
            case "over":
              setHovering(isLikelyImportable(dragged) && isOver(payload.position));
              break;
            case "drop":
              setHovering(false);
              dragged = [];
              if (isOver(payload.position)) {
                drop(payload.paths, toCssPoint(payload.position, window.devicePixelRatio));
              }
              break;
            case "leave":
              setHovering(false);
              dragged = [];
              break;
          }
        }),
      )
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(logError);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [target]);

  return hovering;
}
