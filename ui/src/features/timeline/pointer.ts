/**
 * Pointer interaction on the timeline canvas (timeline design §12.2): seek on the ruler (exact on
 * press, scrub while dragging, exact on release; playback.md §8), select
 * clips, a gap (§4.6) or a marquee on the lanes, move and trim clips as ADR-0006 gestures. The values come from the
 * pure functions in `editDrag.ts`; this module only wires pointer events, stores and IPC.
 *
 * A press becomes a drag only past `DRAG_THRESHOLD`, so a click never sends a gesture. Once a
 * drag starts, `update` goes out at most once per frame (through `beginGesture`) and `commit` on
 * pointer-up; Escape, blur and lost capture cancel (gesture.ts and `lostpointercapture` here).
 */
import { type ClipId, commands, type EditPhase, type Time } from "@/bindings";
import { beginGesture, type Gesture } from "@/ipc/gesture";
import { frameDuration } from "@/lib/time";
import { playheadStore, seekTo } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { clearSelection, selectionStore, setGapSelection, setSelection } from "@/stores/selectionStore";
import { useUiStore } from "@/stores/uiStore";

import { type DragContext, moveValue, type MoveValue, planMove, planTrim, trimValue } from "./editDrag";
import { gapOnTrack } from "./gap";
import { xToTime, type Viewport } from "./geometry";
import { clipsInRect, type Hit, hitTest, pastThreshold, type Rect } from "./hitTest";
import { type ClickModifiers, clickSelection, pressSelection, withLinked } from "./selection";

/** What the canvas draws on top of the model while the pointer is busy. */
export type PointerOverlay = { marquee: Rect | null; snapLine: number | null };

export type PointerDeps = {
  viewport: () => Viewport;
  overlay: PointerOverlay;
  requestDraw: () => void;
};

type Drag<V> = {
  gesture: Gesture<V>;
  valueAt: (event: PointerEvent) => V;
  /** The last value Rust accepted, committed instead when the final one was rejected. */
  lastValid: V | null;
  blocked: boolean;
};

type Press = { x0: number; y0: number; mods: ClickModifiers; before: readonly ClipId[] };

type State =
  | { mode: "idle" }
  /** Dragging the playhead on the ruler: the engine follows with scrub seeks (playback.md §8). */
  | { mode: "seek"; last: Time; moved: boolean }
  | ({ mode: "clip"; hit: Extract<Hit, { kind: "clip" }>; locked: boolean } & Press)
  | ({ mode: "lane"; marquee: boolean; track: number | null } & Press)
  | { mode: "move"; drag: Drag<MoveValue> }
  | { mode: "trim"; drag: Drag<number> }
  /** A gesture that Escape or blur cancelled: ignore the pointer until it is released. */
  | { mode: "done" };

const sameValue = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export function attachPointerInteraction(canvas: HTMLCanvasElement, deps: PointerDeps): () => void {
  let state: State = { mode: "idle" };

  const local = (event: PointerEvent) => {
    const rect = canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };
  const project = () => useProjectStore.getState().state?.project;
  const tracks = () => project()?.tracks ?? [];
  const modifiers = (event: PointerEvent): ClickModifiers => ({
    ctrl: event.ctrlKey || event.metaKey,
    shift: event.shiftKey,
  });

  const setOverlay = (next: Partial<PointerOverlay>) => {
    Object.assign(deps.overlay, next);
    deps.requestDraw();
  };

  const setCursor = (cursor: string) => {
    if (canvas.style.cursor !== cursor) canvas.style.cursor = cursor;
  };

  const hoverCursor = (event: PointerEvent) => {
    const { x, y } = local(event);
    const hit = hitTest(x, y, tracks(), deps.viewport());
    const locked = hit.kind === "clip" && tracks()[hit.track]?.locked;
    setCursor(hit.kind === "clip" && hit.zone !== "body" && !locked ? "ew-resize" : "");
  };

  /** Context from the drag-start state; the snapping preference is read per event for Alt. */
  const dragContext = (): Omit<DragContext, "snapping"> | null => {
    const p = project();
    if (!p) return null;
    return {
      tracks: p.tracks,
      mainTrack: p.mainTrack,
      viewport: deps.viewport(),
      playhead: playheadStore.getState().time,
      frame: frameDuration(p.settings.frameRate),
    };
  };
  const snapping = (event: PointerEvent) => useUiStore.getState().snapping && !event.altKey;

  /** A gesture whose sender remembers whether Rust accepted the last value. */
  function startDrag<V>(
    send: (value: V, phase: EditPhase) => Promise<unknown>,
    valueAt: (event: PointerEvent) => V,
    cursor: string,
  ): Drag<V> {
    const drag: Drag<V> = {
      gesture: beginGesture<V>((value, phase) =>
        send(value, phase).then(
          (result) => {
            if (phase === "update") {
              drag.lastValid = value;
              drag.blocked = false;
              setCursor(cursor);
            }
            return result;
          },
          (error: unknown) => {
            if (phase === "update") {
              drag.blocked = true;
              setCursor("not-allowed");
            }
            throw error;
          },
        ),
      ),
      valueAt,
      lastValid: null,
      blocked: false,
    };
    setCursor(cursor);
    return drag;
  }

  /** Sends `update` when the value changed since the last one. */
  let lastSent: unknown = undefined;
  function updateDrag<V>(drag: Drag<V>, event: PointerEvent) {
    const value = drag.valueAt(event);
    if (sameValue(value, lastSent)) return;
    lastSent = value;
    drag.gesture.update(value);
  }

  function finishDrag<V>(drag: Drag<V>, event: PointerEvent) {
    const final = drag.valueAt(event);
    // Commit what the timeline shows: the last accepted draft when the final value was rejected.
    const value = drag.blocked && drag.lastValid !== null ? drag.lastValid : final;
    void drag.gesture.commit(value);
  }

  const startMove = (s: Extract<State, { mode: "clip" }>): State => {
    const ctx = dragContext();
    const plan = ctx && planMove(ctx.tracks, selectionStore.getState().clips, s.hit.clip.id);
    if (!ctx || !plan) return { mode: "done" };
    const valueAt = (event: PointerEvent) => {
      const { x, y } = local(event);
      const result = moveValue(plan, { ...ctx, snapping: snapping(event) }, x - s.x0, y);
      setOverlay({ snapLine: result.snap });
      return result.value;
    };
    const drag = startDrag<MoveValue>(
      (v, phase) => commands.moveClips(plan.clips, plan.anchor, v.start, v.track, phase),
      valueAt,
      "grabbing",
    );
    return { mode: "move", drag };
  };

  const startTrim = (s: Extract<State, { mode: "clip" }>): State => {
    const ctx = dragContext();
    const edge = s.hit.zone === "start" ? "start" : "end";
    const plan = ctx && planTrim(ctx.tracks, s.hit.clip.id, edge);
    if (!ctx || !plan) return { mode: "done" };
    const valueAt = (event: PointerEvent) => {
      const result = trimValue(plan, { ...ctx, snapping: snapping(event) }, local(event).x - s.x0);
      setOverlay({ snapLine: result.snap });
      return result.time;
    };
    const drag = startDrag<number>(
      (time, phase) => commands.trimClip(plan.clip, plan.edge, time, phase),
      valueAt,
      "ew-resize",
    );
    return { mode: "trim", drag };
  };

  const marqueeTo = (s: Extract<State, { mode: "lane" }>, event: PointerEvent) => {
    const { x, y } = local(event);
    const marquee = { x0: s.x0, y0: s.y0, x1: x, y1: y };
    const inside = clipsInRect(marquee, tracks(), deps.viewport());
    const base = s.mods.ctrl || s.mods.shift ? s.before : [];
    setSelection(withLinked(tracks(), [...base, ...inside]));
    setOverlay({ marquee });
  };

  /** A plain click on an empty lane spot selects the gap there (§4.6), or clears the selection. */
  const selectGapOrClear = (s: Extract<State, { mode: "lane" }>) => {
    const track = s.track === null ? undefined : tracks()[s.track];
    const at = Math.max(0, xToTime(s.x0, deps.viewport()));
    if (track && gapOnTrack(track, at)) setGapSelection({ track: track.id, at });
    else clearSelection();
  };

  const reset = () => {
    state = { mode: "idle" };
    lastSent = undefined;
    setCursor("");
    if (deps.overlay.marquee !== null || deps.overlay.snapLine !== null)
      setOverlay({ marquee: null, snapLine: null });
  };

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || state.mode !== "idle") return;
    const { x, y } = local(event);
    canvas.setPointerCapture(event.pointerId);
    const hit = hitTest(x, y, tracks(), deps.viewport());
    const press: Press = {
      x0: x,
      y0: y,
      mods: modifiers(event),
      before: selectionStore.getState().clips,
    };
    if (hit.kind === "ruler") {
      const time = xToTime(x, deps.viewport());
      state = { mode: "seek", last: time, moved: false };
      seekTo(time, "exact");
    } else if (hit.kind === "clip") {
      setSelection(pressSelection(tracks(), press.before, hit.clip.id, press.mods));
      state = { mode: "clip", hit, locked: tracks()[hit.track]?.locked ?? true, ...press };
    } else {
      state = { mode: "lane", marquee: false, track: hit.track, ...press };
    }
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!canvas.hasPointerCapture(event.pointerId)) {
      if (state.mode === "idle") hoverCursor(event);
      return;
    }
    const s = state;
    switch (s.mode) {
      case "seek": {
        const time = xToTime(local(event).x, deps.viewport());
        state = { ...s, last: time, moved: true };
        seekTo(time, "scrub");
        return;
      }
      case "clip": {
        const { x, y } = local(event);
        if (s.locked || !pastThreshold(x - s.x0, y - s.y0)) return;
        state = s.hit.zone === "body" ? startMove(s) : startTrim(s);
        onPointerMove(event);
        return;
      }
      case "lane": {
        const { x, y } = local(event);
        if (!s.marquee && !pastThreshold(x - s.x0, y - s.y0)) return;
        state = { ...s, marquee: true };
        marqueeTo(s, event);
        return;
      }
      case "move":
      case "trim":
        if (!s.drag.gesture.active) {
          state = { mode: "done" };
          setOverlay({ snapLine: null });
          setCursor("");
          return;
        }
        // Split so TypeScript infers each drag's value type.
        if (s.mode === "move") updateDrag(s.drag, event);
        else updateDrag(s.drag, event);
        return;
      default:
        return;
    }
  };

  const onPointerUp = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const s = state;
    if (s.mode === "seek") {
      // The drag ends on the exact frame; a plain click already sent it.
      if (s.moved) seekTo(xToTime(local(event).x, deps.viewport()), "exact");
    } else if (s.mode === "clip") {
      setSelection(clickSelection(tracks(), s.before, s.hit.clip.id, s.mods));
    } else if (s.mode === "lane" && !s.marquee) {
      if (!s.mods.ctrl && !s.mods.shift) selectGapOrClear(s);
    } else if (s.mode === "move") {
      finishDrag(s.drag, event);
    } else if (s.mode === "trim") {
      finishDrag(s.drag, event);
    }
    reset();
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  };

  /** Capture lost (another window, a system dialog) or pointer cancelled: abandon the gesture. */
  const onLost = () => {
    const s = state;
    if (s.mode === "move" || s.mode === "trim") void s.drag.gesture.cancel();
    if (s.mode === "seek" && s.moved) seekTo(s.last, "exact");
    if (s.mode !== "idle") reset();
  };

  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("pointercancel", onLost);
  canvas.addEventListener("lostpointercapture", onLost);
  return () => {
    onLost();
    canvas.removeEventListener("pointerdown", onPointerDown);
    canvas.removeEventListener("pointermove", onPointerMove);
    canvas.removeEventListener("pointerup", onPointerUp);
    canvas.removeEventListener("pointercancel", onLost);
    canvas.removeEventListener("lostpointercapture", onLost);
  };
}
