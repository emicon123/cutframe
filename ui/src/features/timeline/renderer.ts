/** Draws the timeline lanes, ruler, clips and playhead onto a canvas. No React in here. */
import type {
  ClipId,
  ClipView,
  MediaId,
  MediaView_Serialize,
  TrackId,
  TrackKind,
  TrackView,
} from "@/bindings";

import {
  clipSourceMap,
  drawClipWaveform,
  drawFilmstrip,
  type PeakLevels,
  type TileSource,
} from "./clipVisuals";
import type { DropPreview } from "./drop";
import { normalizeRect, type Rect } from "./hitTest";
import { clipBadges } from "./retime";
import { trackDimmed } from "./tracks";
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
  /** Waveform bars on clips. */
  wave: string;
  /** Strip behind the waveform under a video clip's filmstrip. */
  waveBand: string;
  /** Shade behind the clip labels at the top of a filmstrip. */
  labelShade: string;
  /** Pill behind the speed, reverse and freeze badges (text in `clipText`). */
  badge: string;
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
    wave: v("--clip-wave"),
    waveBand: v("--clip-wave-band"),
    labelShade: v("--clip-label-shade"),
    badge: v("--clip-badge"),
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
  /** The selected gap: its track index and span. */
  gap: { index: number; start: number; end: number } | null;
  /** The rubber band of a marquee selection, in canvas CSS px. */
  marquee: Rect | null;
  /** Time of the snap indicator while a move or trim snapped. */
  snapLine: number | null;
  /** Label text for a clip's length. */
  durationText: (flicks: number) => string;
  /** The project's media by id, for clip filmstrips and waveforms. */
  media: ReadonlyMap<MediaId, MediaView_Serialize>;
  /** Decoded `media://` products; a miss queues a load and draws nothing (F1.34, F1.35). */
  products: ClipProducts;
};

export type ClipProducts = {
  tiles: TileSource<MediaView_Serialize>;
  waveforms: { get(item: MediaView_Serialize, index: number): PeakLevels | null };
};

/** Opacity of the clips on a muted or hidden track (`trackDimmed`). */
const DIMMED_ALPHA = 0.4;
/** Clips narrower than this get no link marker. */
const LINK_MARK_MIN_WIDTH = 40;
/** The clip body inside its lane. */
const BODY_TOP = 3;
const BODY_HEIGHT = TRACK_HEIGHT - 6;
/** Height of the waveform strip under a video clip's filmstrip. */
const WAVE_STRIP_HEIGHT = 16;
/** Height of the label shade over a filmstrip. */
const LABEL_SHADE_HEIGHT = 40;
/** Badge pills after the clip length: height, inner padding and gap. */
const BADGE_HEIGHT = 15;
const BADGE_PADDING = 5;
const BADGE_GAP = 4;

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
    if (scene.gap?.index === index) drawGap(ctx, scene.gap, top, scene);
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
  const alpha = trackDimmed(track) ? DIMMED_ALPHA : 1;
  for (const clip of track.clips) {
    const x = timeToX(clip.start, viewport);
    const w = clip.duration / viewport.flicksPerPixel;
    if (x > width || x + w < 0) continue;
    const { source } = clip;
    const fromMedia = source.kind === "media" || source.kind === "freeze";
    const offline = fromMedia && scene.offline.has(source.media);
    ctx.globalAlpha = alpha;
    ctx.fillStyle = offline ? theme.offline : clipFill(clip, track.kind, theme);
    ctx.beginPath();
    ctx.roundRect(x + 0.5, top + 3, Math.max(2, w - 1), TRACK_HEIGHT - 6, 6);
    ctx.fill();
    ctx.save();
    ctx.clip();
    if (offline) drawHatch(ctx, x, top, w, theme.invalid);
    else if (fromMedia) drawClipMedia(ctx, clip, source.media, track.kind, x, w, top, scene);
    // F1.36: the name, and the length under it (with the offline note for a missing file).
    ctx.fillStyle = theme.clipText;
    ctx.font = "500 12px 'Inter Variable', sans-serif";
    ctx.textBaseline = "top";
    const left = Math.max(x, 0) + 8;
    ctx.fillText(clip.name, left, top + 9);
    const duration = scene.durationText(clip.duration);
    ctx.font = "500 11px 'Inter Variable', sans-serif";
    ctx.globalAlpha = 0.75 * alpha;
    ctx.fillText(duration, left, top + 27);
    ctx.globalAlpha = alpha;
    let after = left + ctx.measureText(duration).width + 8;
    if (offline) {
      ctx.fillStyle = theme.invalid;
      ctx.font = "600 11px 'Inter Variable', sans-serif";
      ctx.fillText(scene.offlineLabel, after, top + 27);
      after += ctx.measureText(scene.offlineLabel).width + 8;
    }
    drawBadges(ctx, clipBadges(clip.source), after, top + 25, Math.min(x + w, width) - 6, theme);
    if (clip.link && w >= LINK_MARK_MIN_WIDTH)
      drawLinkMark(ctx, Math.min(x + w, width) - 10, top + 13, theme.clipText);
    ctx.restore();
    ctx.globalAlpha = 1;
    if (scene.selected.has(clip.id)) {
      ctx.strokeStyle = theme.selected;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.roundRect(x + 1.5, top + 4, Math.max(2, w - 3), TRACK_HEIGHT - 8, 5);
      ctx.stroke();
    }
  }
}

/**
 * The filmstrip (video and freeze-frame clips) and waveform (audio clips, and video clips that
 * still play their own sound) of a media clip, over its used source range (`clipSourceMap`: the
 * speed stretches it, reverse mirrors it, a freeze frame repeats one frame). The clip's rounded
 * rectangle is the clip path already. Products not loaded yet are skipped and drawn when they
 * arrive.
 */
function drawClipMedia(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  media: MediaId,
  kind: TrackKind,
  x: number,
  w: number,
  top: number,
  scene: Scene,
) {
  const item = scene.media.get(media);
  const map = clipSourceMap(clip);
  if (!item || !map) return;
  const { viewport, width, theme, products } = scene;
  const filmstrip = kind === "video" && item.kind === "video" ? item.filmstrip : null;
  const sound = item.audio !== null && (kind === "audio" || (kind === "video" && clip.audio != null));
  const bodyTop = top + BODY_TOP;
  const left = Math.max(x, 0);
  const visible = Math.min(x + w, width) - left;
  const strip = filmstrip !== null;
  if (filmstrip) {
    const height = sound ? BODY_HEIGHT - WAVE_STRIP_HEIGHT : BODY_HEIGHT;
    drawFilmstrip(ctx, filmstrip, products.tiles, item, map, x, w, bodyTop, height, viewport, width);
  }
  const peaks = sound ? products.waveforms.get(item, 0) : null;
  if (sound) {
    const waveTop = strip ? bodyTop + BODY_HEIGHT - WAVE_STRIP_HEIGHT : bodyTop;
    const waveHeight = strip ? WAVE_STRIP_HEIGHT : BODY_HEIGHT;
    if (strip) {
      ctx.fillStyle = theme.waveBand;
      ctx.fillRect(left, waveTop, visible, waveHeight);
    }
    if (peaks) {
      ctx.fillStyle = theme.wave;
      drawClipWaveform(ctx, peaks, map, x, w, waveTop + 1, waveHeight - 2, viewport, width);
    }
  }
  // Keeps the name and length readable over the pictures.
  if (strip || peaks) {
    ctx.save();
    ctx.translate(0, bodyTop);
    ctx.fillStyle = labelShade(ctx, theme.labelShade);
    ctx.fillRect(left, 0, visible, LABEL_SHADE_HEIGHT);
    ctx.restore();
  }
}

/**
 * The speed, reverse and freeze badges as pills from `x` on the length line, while they fit
 * before `right` (a narrow clip shows fewer or none).
 */
function drawBadges(
  ctx: CanvasRenderingContext2D,
  badges: readonly string[],
  x: number,
  y: number,
  right: number,
  theme: TimelineTheme,
) {
  if (badges.length === 0) return;
  ctx.font = "600 10px 'Inter Variable', sans-serif";
  ctx.textBaseline = "middle";
  let left = x;
  for (const badge of badges) {
    const w = ctx.measureText(badge).width + 2 * BADGE_PADDING;
    if (left + w > right) break;
    ctx.fillStyle = theme.badge;
    ctx.beginPath();
    ctx.roundRect(left, y, w, BADGE_HEIGHT, 4);
    ctx.fill();
    ctx.fillStyle = theme.clipText;
    ctx.fillText(badge, left + BADGE_PADDING, y + BADGE_HEIGHT / 2 + 0.5);
    left += w + BADGE_GAP;
  }
  ctx.textBaseline = "top";
}

/** The label shade gradient, in coordinates relative to the clip body's top; built once per colour. */
const labelShades = new WeakMap<CanvasRenderingContext2D, { color: string; gradient: CanvasGradient }>();

function labelShade(ctx: CanvasRenderingContext2D, color: string): CanvasGradient {
  const cached = labelShades.get(ctx);
  if (cached?.color === color) return cached.gradient;
  const gradient = ctx.createLinearGradient(0, 0, 0, LABEL_SHADE_HEIGHT);
  gradient.addColorStop(0, color);
  gradient.addColorStop(1, "transparent");
  labelShades.set(ctx, { color, gradient });
  return gradient;
}

/** Two interlocking rings at the clip's top-right corner: the clip is linked (§3.5). */
function drawLinkMark(ctx: CanvasRenderingContext2D, right: number, y: number, color: string) {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.globalAlpha *= 0.85;
  ctx.lineWidth = 1.5;
  for (const cx of [right - 7, right - 2]) {
    ctx.beginPath();
    ctx.roundRect(cx - 4, y - 2.5, 8, 5, 2.5);
    ctx.stroke();
  }
  ctx.restore();
}

/** The selected gap: a translucent fill with an outline in the selection colour. */
function drawGap(
  ctx: CanvasRenderingContext2D,
  gap: { start: number; end: number },
  top: number,
  { viewport, theme }: Scene,
) {
  const x = timeToX(gap.start, viewport);
  const w = (gap.end - gap.start) / viewport.flicksPerPixel;
  ctx.save();
  ctx.fillStyle = theme.selected;
  ctx.strokeStyle = theme.selected;
  ctx.globalAlpha = 0.18;
  ctx.fillRect(x, top + 3, w, TRACK_HEIGHT - 6);
  ctx.globalAlpha = 1;
  ctx.lineWidth = 1;
  ctx.setLineDash([4, 3]);
  ctx.strokeRect(Math.round(x) + 0.5, top + 3.5, Math.max(1, Math.round(w) - 1), TRACK_HEIGHT - 7);
  ctx.restore();
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
