import { Film, Image, ImageOff, Music } from "lucide-react";
import { type PointerEvent, type RefObject, useEffect, useRef, useState } from "react";

import { FILMSTRIP_FRAME_HEIGHT, type MediaKind, type MediaView_Serialize } from "@/bindings";
import { t } from "@/i18n";
import {
  fetchMediaBytes,
  filmstripTileUrl,
  parseWaveform,
  posterUrl,
  probeMedia,
  retryDelay,
  type Waveform,
  waveformUrl,
} from "@/ipc/media";
import { cn } from "@/lib/utils";

import { previewKey } from "./relink";
import { type SkimFrame, skimFrame, tileBackground } from "./skim";
import { drawWaveform } from "./waveformDraw";

export const KIND_ICONS = { video: Film, audio: Music, image: Image } satisfies Record<MediaKind, unknown>;

/**
 * The picture of a media item: the poster (video, image) with hover-scrub over the filmstrip
 * (video), or a waveform (audio). Everything comes from `media://`; nothing is decoded here.
 */
export function Thumbnail({ item, className }: { item: MediaView_Serialize; className?: string }) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-sm bg-muted",
        item.status === "offline" && "opacity-50",
        className,
      )}
    >
      {item.kind === "audio" ? (
        <WaveformThumb key={previewKey(waveformUrl(item), item.status)} item={item} />
      ) : (
        <Poster key={previewKey(posterUrl(item), item.status)} item={item} />
      )}
      {item.kind === "video" && item.filmstrip && <Skim item={item} layout={item.filmstrip} />}
    </div>
  );
}

function Placeholder({ kind, failed }: { kind: MediaKind; failed: boolean }) {
  const Icon = failed ? ImageOff : KIND_ICONS[kind];
  return (
    <div className="absolute inset-0 flex items-center justify-center text-muted-foreground">
      <Icon className="size-5 opacity-60" aria-hidden />
      {failed && <span className="sr-only">{t.mediaBin.previewUnavailable}</span>}
    </div>
  );
}

type PosterPhase = { phase: "loading" | "loaded" | "error" | "failed"; attempt: number };

/**
 * `<img loading="lazy">` of the poster. An `<img>` error carries no status, so a failed load asks
 * the URL once more with `fetch`: `503` retries with backoff (the request is re-made by remounting
 * the image), `500` and `404` show the placeholder for good.
 */
function Poster({ item }: { item: MediaView_Serialize }) {
  const url = posterUrl(item);
  const [state, setState] = useState<PosterPhase>({ phase: "loading", attempt: 0 });

  useEffect(() => {
    if (state.phase !== "error") return;
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    probeMedia(url, abort.signal)
      .then((status) => {
        const delay = status === "retry" || status === "ready" ? retryDelay(state.attempt) : null;
        if (delay === null) setState({ phase: "failed", attempt: state.attempt });
        else timer = setTimeout(() => setState({ phase: "loading", attempt: state.attempt + 1 }), delay);
      })
      .catch(() => undefined);
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [state, url]);

  if (state.phase === "failed") return <Placeholder kind={item.kind} failed />;
  return (
    <>
      {state.phase !== "loaded" && <Placeholder kind={item.kind} failed={false} />}
      <img
        key={state.attempt}
        src={url}
        alt=""
        loading="lazy"
        decoding="async"
        draggable={false}
        className={cn("absolute inset-0 size-full object-contain", state.phase !== "loaded" && "opacity-0")}
        onLoad={() => setState((s) => ({ phase: "loaded", attempt: s.attempt }))}
        onError={() => setState((s) => ({ phase: "error", attempt: s.attempt }))}
      />
    </>
  );
}

/**
 * Hover-scrub (skimming): the pointer's x position picks a filmstrip frame, shown as a CSS sprite
 * of its tile over the poster. Only this element re-renders, and only when the frame changes.
 */
function Skim({
  item,
  layout,
}: {
  item: MediaView_Serialize;
  layout: NonNullable<MediaView_Serialize["filmstrip"]>;
}) {
  const [skim, setSkim] = useState<{ frame: SkimFrame; fraction: number } | null>(null);

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerType === "touch") return;
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width <= 0) return;
    const fraction = (event.clientX - rect.left) / rect.width;
    const frame = skimFrame(layout, fraction);
    if (frame && frame.frame !== skim?.frame.frame) setSkim({ frame, fraction });
  };

  return (
    <div
      className="absolute inset-0 flex justify-center"
      data-testid="skim"
      onPointerMove={onPointerMove}
      onPointerLeave={() => setSkim(null)}
    >
      {skim && (
        <>
          <div
            className="h-full max-w-full bg-no-repeat"
            style={{
              aspectRatio: `${layout.frameWidth} / ${FILMSTRIP_FRAME_HEIGHT}`,
              backgroundImage: `url("${filmstripTileUrl(item, skim.frame.tile)}")`,
              ...tileBackground(skim.frame),
            }}
          />
          <div
            className="pointer-events-none absolute inset-y-0 w-px bg-primary"
            style={{ left: `${Math.min(1, Math.max(0, skim.fraction)) * 100}%` }}
          />
        </>
      )}
    </div>
  );
}

/** Waveforms already fetched, by URL; switching views or scrolling back does not refetch. */
const waveforms = new Map<string, Promise<Waveform | null>>();
const WAVEFORM_CACHE_LIMIT = 256;

function loadWaveform(url: string): Promise<Waveform | null> {
  let pending = waveforms.get(url);
  if (!pending) {
    pending = fetchMediaBytes(url).then((bytes) => {
      const waveform = bytes ? parseWaveform(bytes) : null;
      // Not remembered when missing, so a later mount (e.g. after the file is back) asks again.
      if (!waveform) waveforms.delete(url);
      return waveform;
    });
    waveforms.set(url, pending);
    // Maps iterate in insertion order: drop the oldest entry.
    if (waveforms.size > WAVEFORM_CACHE_LIMIT) {
      const oldest = waveforms.keys().next().value;
      if (oldest !== undefined) waveforms.delete(oldest);
    }
  }
  return pending;
}

/** True once `element` has scrolled into view (at once where IntersectionObserver is missing). */
function useSeen(element: RefObject<Element | null>): boolean {
  const [seen, setSeen] = useState(() => typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const target = element.current;
    if (seen || !target) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) setSeen(true);
    });
    observer.observe(target);
    return () => observer.disconnect();
  }, [element, seen]);
  return seen;
}

function WaveformThumb({ item }: { item: MediaView_Serialize }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const seen = useSeen(canvas);
  const [state, setState] = useState<"loading" | "ready" | "failed">("loading");
  const url = waveformUrl(item);

  useEffect(() => {
    if (!seen) return;
    let live = true;
    let resize: ResizeObserver | undefined;
    void loadWaveform(url).then((waveform) => {
      const element = canvas.current;
      if (!live || !element) return;
      if (!waveform) {
        setState("failed");
        return;
      }
      setState("ready");
      const draw = () => {
        const ratio = window.devicePixelRatio || 1;
        element.width = Math.max(1, Math.round(element.clientWidth * ratio));
        element.height = Math.max(1, Math.round(element.clientHeight * ratio));
        const color = getComputedStyle(element).getPropertyValue("--clip-audio").trim();
        drawWaveform(element, waveform.peaks, color);
      };
      draw();
      if (typeof ResizeObserver !== "undefined") {
        resize = new ResizeObserver(draw);
        resize.observe(element);
      }
    });
    return () => {
      live = false;
      resize?.disconnect();
    };
  }, [seen, url]);

  return (
    <>
      {state !== "ready" && <Placeholder kind="audio" failed={state === "failed"} />}
      <canvas ref={canvas} className="absolute inset-0 size-full" aria-hidden />
    </>
  );
}
