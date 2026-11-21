/** Draws the timeline lanes, ruler, clips and playhead onto a canvas. No React in here. */
import type { TrackKind, TrackView } from "@/bindings";

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
  ruler: string;
  rulerText: string;
  tick: string;
  playhead: string;
  clipText: string;
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
    ruler: v("--ruler"),
    rulerText: v("--muted-foreground"),
    tick: v("--border"),
    playhead: v("--playhead"),
    clipText: v("--foreground"),
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
};

export function drawTimeline(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { width, height, tracks, viewport, theme } = scene;
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, width, height);

  tracks.forEach((track, index) => {
    const top = trackTop(index);
    ctx.fillStyle = index % 2 === 0 ? theme.track : theme.trackAlt;
    ctx.fillRect(0, top, width, TRACK_HEIGHT);
    drawClips(ctx, track, top, scene);
  });

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

function drawClips(
  ctx: CanvasRenderingContext2D,
  track: TrackView,
  top: number,
  { width, viewport, theme }: Scene,
) {
  for (const clip of track.clips) {
    const x = timeToX(clip.start, viewport);
    const w = clip.duration / viewport.flicksPerPixel;
    if (x > width || x + w < 0) continue;
    ctx.fillStyle = theme.clip[track.kind];
    ctx.beginPath();
    ctx.roundRect(x + 0.5, top + 3, Math.max(2, w - 1), TRACK_HEIGHT - 6, 6);
    ctx.fill();
    ctx.save();
    ctx.clip();
    ctx.fillStyle = theme.clipText;
    ctx.font = "500 12px 'Inter Variable', sans-serif";
    ctx.textBaseline = "top";
    ctx.fillText(clip.name, Math.max(x, 0) + 8, top + 9);
    ctx.restore();
  }
}
