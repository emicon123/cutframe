/** Draws the timeline lanes, ruler, clips and playhead onto a canvas. No React in here. */
import type { ClipId, ClipView, MediaId, TrackId, TrackKind, TrackView } from "@/bindings";

import type { DropPreview } from "./drop";
import { normalizeRect, type Rect } from "./hitTest";
import {
  RULER_HEIGHT,
  TRACK_HEIGHT,
  formatRulerLabel,
  rulerStep,
  timeToX,
  trackTop,
  type Viewport,
} from "./geometry";

export type TimelineTheme = {
  background: string;
  track: string;
  trackAlt: string;
  /** Lane of the main track, drawn distinctly. */
  trackMain: string;
  accent: string;
  invalid: string;
  offline: string;
  ruler: string;
  rulerText: string;
  tick: string;
  playhead: string;
  clipText: string;
  selected: string;
  snapLine: string;
  clip: Record<TrackKind, string>;
};

/** Reads the colour tokens from CSS custom properties defined in `index.css`. */
export function readTheme(element: Element): TimelineTheme {
  const style = getComputedStyle(element);
  const v = (name: string) => style.getPropertyValue(name).trim();
  return {
    background: v("--timeline"),
    track: v("--track"),
    trackAlt: v("--track-alt"),
    trackMain: v("--track-main"),
    accent: v("--primary"),
    invalid: v("--destructive"),
    offline: v("--muted"),
    ruler: v("--ruler"),
    rulerText: v("--muted-foreground"),
    tick: v("--border"),
    playhead: v("--playhead"),
    clipText: v("--foreground"),
    selected: v("--clip-selected"),
    snapLine: v("--snap-line"),
    clip: { video: v("--clip-video"), audio: v("--clip-audio"), text: v("--clip-text") },
  };
}

export type Scene = {
  width: number;
  height: number;
  tracks: TrackView[];
  viewport: Viewport;
  playhead: number;
  theme: TimelineTheme;
  mainTrack: TrackId | null;
  /** Media whose file is missing; their clips get the offline look. */
  offline: ReadonlySet<MediaId>;
  offlineLabel: string;
  /** Where dragged media would land, while a drag is over the timeline. */
  drop: DropPreview | null;
  selected: ReadonlySet<ClipId>;
  /** The rubber band of a marquee selection, in canvas CSS px. */
  marquee: Rect | null;
  /** Time of the snap indicator while a move or trim snapped. */
  snapLine: number | null;
  /** Label text for a clip's length. */
  durationText: (flicks: number) => string;
};

export function drawTimeline(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { width, height, tracks, viewport, theme } = scene;
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, width, height);

  tracks.forEach((track, index) => {
    const top = trackTop(index);
    const main = track.id === scene.mainTrack;
    ctx.fillStyle = main ? theme.trackMain : index % 2 === 0 ? theme.track : theme.trackAlt;
    ctx.fillRect(0, top, width, TRACK_HEIGHT);
    if (main) {
      ctx.fillStyle = theme.accent;
      ctx.fillRect(0, top, 3, TRACK_HEIGHT);
    }
    drawClips(ctx, track, top, scene);
  });

  if (scene.drop) drawDrop(ctx, scene.drop, scene);
  if (scene.snapLine !== null) drawSnapLine(ctx, scene.snapLine, scene);
  if (scene.marquee) drawMarquee(ctx, scene.marquee, theme);
  drawRuler(ctx, scene);

  const x = Math.round(timeToX(scene.playhead, viewport)) + 0.5;
  if (x >= 0 && x <= width) {
    ctx.strokeStyle = theme.playhead;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, height);
    ctx.stroke();
    ctx.fillStyle = theme.playhead;
    ctx.beginPath();
    ctx.moveTo(x - 6, 0);
    ctx.lineTo(x + 6, 0);
    ctx.lineTo(x + 6, 8);
    ctx.lineTo(x, 14);
    ctx.lineTo(x - 6, 8);
    ctx.closePath();
    ctx.fill();
  }
}

function drawRuler(ctx: CanvasRenderingContext2D, { width, viewport, theme }: Scene) {
  ctx.fillStyle = theme.ruler;
  ctx.fillRect(0, 0, width, RULER_HEIGHT);

  const step = rulerStep(viewport.flicksPerPixel);
  const minor = step / 5;
  const first = Math.floor(viewport.scroll / minor) * minor;
  const end = viewport.scroll + width * viewport.flicksPerPixel;

  ctx.strokeStyle = theme.tick;
  ctx.fillStyle = theme.rulerText;
  ctx.font = "11px 'Inter Variable', sans-serif";
  ctx.textBaseline = "top";
  ctx.beginPath();
  for (let time = first, i = Math.round(first / minor); time <= end; time += minor, i++) {
    const x = Math.round(timeToX(time, viewport)) + 0.5;
    const major = i % 5 === 0;
    ctx.moveTo(x, major ? 12 : RULER_HEIGHT - 6);
    ctx.lineTo(x, RULER_HEIGHT);
    if (major) ctx.fillText(formatRulerLabel(time, step), x + 4, 4);
  }
  ctx.stroke();
}

function clipFill(clip: ClipView, kind: TrackKind, theme: TimelineTheme): string {
  // Colour cards show their colour; `#rrggbbaa` is valid CSS.
  return clip.source.kind === "color" ? clip.source.color : theme.clip[kind];
}

function drawClips(ctx: CanvasRenderingContext2D, track: TrackView, top: number, scene: Scene) {
  const { width, viewport, theme } = scene;
  for (const clip of track.clips) {
    const x = timeToX(clip.start, viewport);
    const w = clip.duration / viewport.flicksPerPixel;
    if (x > width || x + w < 0) continue;
    const offline = clip.source.kind === "media" && scene.offline.has(clip.source.media);
    ctx.fillStyle = offline ? theme.offline : clipFill(clip, track.kind, theme);
    ctx.beginPath();
    ctx.roundRect(x + 0.5, top + 3, Math.max(2, w - 1), TRACK_HEIGHT - 6, 6);
    ctx.fill();
    ctx.save();
    ctx.clip();
    if (offline) drawHatch(ctx, x, top, w, theme.invalid);
    // F1.36: the name, and the length under it (with the offline note for a missing file).
    ctx.fillStyle = theme.clipText;
    ctx.font = "500 12px 'Inter Variable', sans-serif";
    ctx.textBaseline = "top";
    const left = Math.max(x, 0) + 8;
    ctx.fillText(clip.name, left, top + 9);
    const duration = scene.durationText(clip.duration);
    ctx.font = "500 11px 'Inter Variable', sans-serif";
    ctx.globalAlpha = 0.75;
    ctx.fillText(duration, left, top + 27);
    ctx.globalAlpha = 1;
    if (offline) {
      ctx.fillStyle = theme.invalid;
      ctx.font = "600 11px 'Inter Variable', sans-serif";
      ctx.fillText(scene.offlineLabel, left + ctx.measureText(duration).width + 8, top + 27);
    }
    ctx.restore();
    if (scene.selected.has(clip.id)) {
      ctx.strokeStyle = theme.selected;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.roundRect(x + 1.5, top + 4, Math.max(2, w - 3), TRACK_HEIGHT - 8, 5);
      ctx.stroke();
    }
  }
}

/** Diagonal stripes: the offline placeholder look (clip path already set). */
function drawHatch(ctx: CanvasRenderingContext2D, x: number, top: number, w: number, color: string) {
  ctx.strokeStyle = color;
  ctx.globalAlpha = 0.35;
  ctx.lineWidth = 2;
  ctx.beginPath();
  for (let sx = Math.floor(x / 12) * 12 - TRACK_HEIGHT; sx < x + w; sx += 12) {
    ctx.moveTo(sx, top + TRACK_HEIGHT);
    ctx.lineTo(sx + TRACK_HEIGHT, top);
  }
  ctx.stroke();
  ctx.globalAlpha = 1;
}

/** The ghost of a drop: a clip outline on the track, or an insertion line for a new track. */
function drawDrop(ctx: CanvasRenderingContext2D, drop: DropPreview, scene: Scene) {
  const { width, viewport, theme } = scene;
  const color = drop.valid ? theme.accent : theme.invalid;
  const x = timeToX(drop.start, viewport);
  const w = Math.max(4, drop.duration / viewport.flicksPerPixel);
  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  if (drop.kind === "track") {
    const top = trackTop(drop.index);
    ctx.globalAlpha = 0.3;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, top + 3, w - 1, TRACK_HEIGHT - 6, 6);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.lineWidth = 2;
    ctx.stroke();
    if (drop.insert) {
      ctx.fillRect(Math.round(x) - 1, top, 3, TRACK_HEIGHT);
    }
  } else {
    const y = trackTop(drop.index) - 1;
    ctx.fillRect(0, y - 1, width, 2);
    ctx.globalAlpha = 0.3;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, y + 2, w - 1, TRACK_HEIGHT / 2, 6);
    ctx.fill();
    ctx.globalAlpha = 1;
  }
  ctx.restore();
  if (drop.snap !== null) drawSnapLine(ctx, drop.snap, scene);
}

/** The dashed line where a drop, move or trim snapped. */
function drawSnapLine(ctx: CanvasRenderingContext2D, time: number, { height, viewport, theme }: Scene) {
  const x = Math.round(timeToX(time, viewport)) + 0.5;
  ctx.save();
  ctx.strokeStyle = theme.snapLine;
  ctx.lineWidth = 1;
  ctx.setLineDash([4, 3]);
  ctx.beginPath();
  ctx.moveTo(x, RULER_HEIGHT);
  ctx.lineTo(x, height);
  ctx.stroke();
  ctx.restore();
}

function drawMarquee(ctx: CanvasRenderingContext2D, rect: Rect, theme: TimelineTheme) {
  const r = normalizeRect(rect);
  ctx.save();
  ctx.fillStyle = theme.selected;
  ctx.strokeStyle = theme.selected;
  ctx.globalAlpha = 0.12;
  ctx.fillRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0);
  ctx.globalAlpha = 0.8;
  ctx.lineWidth = 1;
  ctx.strokeRect(Math.round(r.x0) + 0.5, Math.round(r.y0) + 0.5, r.x1 - r.x0, r.y1 - r.y0);
  ctx.restore();
}
