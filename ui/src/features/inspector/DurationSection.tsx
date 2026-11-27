/**
 * Inspector → Duration (F1.83): the length of an image, colour card or freeze frame, typed in
 * seconds. Sent as a trim of the end edge (`trimClip` with one commit), so the start stays and the
 * same limits as dragging the edge apply.
 */
import { commands, type EditPhase, type Time } from "@/bindings";
import { t } from "@/i18n";
import { beginGesture } from "@/ipc/gesture";
import { formatTimecode, frameDuration } from "@/lib/time";

import { NumberField } from "./fields";
import { anyLocked, type SectionProps } from "./sections";
import { formatDurationSeconds, parseDuration } from "./values";

export function DurationSection({ targets, project }: SectionProps) {
  const [item] = targets;
  if (!item) return null;
  const { clip } = item;
  const rate = project.settings.frameRate;
  const frame = frameDuration(rate);
  const sendEnd = (end: Time, phase: EditPhase) => commands.trimClip(clip.id, "end", end, phase);

  return (
    <>
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs text-muted-foreground tabular-nums">
          {formatTimecode(clip.duration, rate)}
        </span>
        <NumberField
          aria-label={t.inspector.still.field}
          value={clip.duration}
          disabled={anyLocked(targets)}
          format={formatDurationSeconds}
          parse={(text) => parseDuration(text, frame)}
          unit={t.inspector.units.seconds}
          onCommit={(duration) => void beginGesture(sendEnd).commit(clip.start + duration)}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t.inspector.still.hint}</p>
    </>
  );
}
