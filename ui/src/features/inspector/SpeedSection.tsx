/**
 * Inspector → Speed (F1.79–F1.81): constant speed on a logarithmic slider (0.1×–100×) with a
 * number field and presets, preserve pitch, reverse, and the duration the speed results in.
 */
import { type ClipSource, commands, type EditPhase, type Speed } from "@/bindings";
import { Button } from "@/components/ui/button";
import { MAX_SOUNDING_SPEED, NORMAL_SPEED, speedFactorText } from "@/features/timeline/retime";
import { t } from "@/i18n";
import { beginGesture } from "@/ipc/gesture";
import { formatTimecode } from "@/lib/time";
import { run } from "@/stores/appStore";

import { CheckField, SliderField } from "./fields";
import { anyLocked, hasAudio, type SectionProps, shared } from "./sections";
import {
  formatSpeed,
  parseSpeed,
  SPEED_PRESETS,
  SPEED_SLIDER_MAX,
  sliderToSpeed,
  speedToSlider,
} from "./values";

export function SpeedSection({ targets, project }: SectionProps) {
  // `setClipSpeed` and `setClipReverse` expand links in Rust; the selection holds them anyway.
  const ids = targets.map((item) => item.clip.id);
  const disabled = anyLocked(targets);
  // Preserve pitch is an audio property, so it goes to the clips that play sound.
  const sounding = targets.filter(hasAudio);

  const speed = shared(targets, (item) => sourceSpeed(item.clip.source), NORMAL_SPEED);
  const reverse = shared(
    targets,
    (item) => item.clip.source.kind === "media" && item.clip.source.reverse,
    false,
  );
  const pitch = shared(sounding, (item) => item.clip.audio?.preservePitch ?? true, true);
  const duration = shared(targets, (item) => item.clip.duration, 0);
  const muted =
    sounding.length > 0 && targets.some((item) => sourceSpeed(item.clip.source) > MAX_SOUNDING_SPEED);

  const sendSpeed = (next: Speed, phase: EditPhase) => commands.setClipSpeed(ids, next, phase);

  return (
    <>
      <SliderField
        label={t.inspector.speed.speed}
        value={speed.value}
        mixed={speed.mixed}
        disabled={disabled}
        min={0}
        max={SPEED_SLIDER_MAX}
        toSlider={speedToSlider}
        fromSlider={sliderToSpeed}
        format={formatSpeed}
        parse={parseSpeed}
        unit={t.inspector.units.factor}
        send={sendSpeed}
      />
      <div role="group" aria-label={t.inspector.speed.presets} className="flex gap-1">
        {SPEED_PRESETS.map((preset) => {
          const factor = speedFactorText(preset);
          const active = !speed.mixed && speed.value === preset;
          return (
            <Button
              key={preset}
              size="xs"
              variant={active ? "secondary" : "outline"}
              className="flex-1 tabular-nums"
              aria-label={t.inspector.speed.presetLabel(factor)}
              aria-pressed={active}
              disabled={disabled}
              // A preset is a single commit, one history entry.
              onClick={() => void beginGesture(sendSpeed).commit(preset)}
            >
              {t.inspector.speed.preset(factor)}
            </Button>
          );
        })}
      </div>
      <CheckField
        label={t.inspector.speed.preservePitch}
        checked={pitch.value}
        mixed={pitch.mixed}
        disabled={disabled || sounding.length === 0}
        onChange={(value) =>
          void run(
            commands.setClipProperty(
              sounding.map((item) => item.clip.id),
              { property: "preservePitch", value },
              "commit",
            ),
          )
        }
      />
      <CheckField
        label={t.inspector.speed.reverse}
        checked={reverse.value}
        mixed={reverse.mixed}
        disabled={disabled}
        onChange={(value) => void run(commands.setClipReverse(ids, value))}
      />
      <dl className="grid grid-cols-[auto_1fr] gap-x-4">
        <dt className="text-muted-foreground">{t.inspector.speed.duration}</dt>
        <dd className="text-right tabular-nums">
          {duration.mixed ? t.inspector.mixed : formatTimecode(duration.value, project.settings.frameRate)}
        </dd>
      </dl>
      {muted && <p className="text-xs text-muted-foreground">{t.inspector.speed.mutedAbove}</p>}
    </>
  );
}

function sourceSpeed(source: ClipSource): Speed {
  return source.kind === "media" ? source.speed : NORMAL_SPEED;
}
