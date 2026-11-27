/** Inspector → Audio (F1.77, F1.78): volume in percent with a dB readout, fade in and fade out. */
import { commands, type EditPhase, type Time } from "@/bindings";
import { t } from "@/i18n";
import { frameDuration } from "@/lib/time";

import { SliderField } from "./fields";
import { anyLocked, type SectionProps, shared } from "./sections";
import {
  fadeLimit,
  formatDecibels,
  formatFade,
  formatVolume,
  MAX_VOLUME_PERCENT,
  parseFade,
  parseVolume,
  volumeFromSlider,
  volumeToSlider,
} from "./values";

export function AudioSection({ targets, project }: SectionProps) {
  // Properties are per clip: Rust does not expand links here, and the selection already holds
  // whole link groups. Clips without sound are not targets.
  const ids = targets.map((item) => item.clip.id);
  const disabled = anyLocked(targets);
  const frame = frameDuration(project.settings.frameRate);

  const volume = shared(targets, (item) => item.clip.audio?.volume.value ?? 1, 1);
  const fadeIn = shared(targets, (item) => item.clip.audio?.fadeIn ?? 0, 0);
  const fadeOut = shared(targets, (item) => item.clip.audio?.fadeOut ?? 0, 0);
  // Display only: the slider ends where the longest clip runs out of room; Rust clamps each clip.
  const fadeInMax = Math.max(...targets.map((i) => fadeLimit(i.clip.duration, i.clip.audio?.fadeOut ?? 0)));
  const fadeOutMax = Math.max(...targets.map((i) => fadeLimit(i.clip.duration, i.clip.audio?.fadeIn ?? 0)));

  const sendVolume = (gain: number, phase: EditPhase) =>
    commands.setClipProperty(ids, { property: "volume", value: gain }, phase);
  const sendFadeIn = (fade: Time, phase: EditPhase) =>
    commands.setClipProperty(ids, { property: "fadeIn", value: Math.round(fade) }, phase);
  const sendFadeOut = (fade: Time, phase: EditPhase) =>
    commands.setClipProperty(ids, { property: "fadeOut", value: Math.round(fade) }, phase);

  return (
    <>
      <SliderField
        label={t.inspector.audio.volume}
        value={volume.value}
        mixed={volume.mixed}
        disabled={disabled}
        min={0}
        max={MAX_VOLUME_PERCENT}
        toSlider={volumeToSlider}
        fromSlider={volumeFromSlider}
        format={formatVolume}
        parse={parseVolume}
        unit={t.inspector.units.percent}
        readout={volume.mixed ? t.inspector.mixed : formatDecibels(volume.value)}
        send={sendVolume}
      />
      <SliderField
        label={t.inspector.audio.fadeIn}
        value={fadeIn.value}
        mixed={fadeIn.mixed}
        disabled={disabled || fadeInMax <= 0}
        min={0}
        max={Math.max(fadeInMax, frame)}
        step={frame}
        format={formatFade}
        parse={(text) => parseFade(text, fadeInMax)}
        unit={t.inspector.units.seconds}
        send={sendFadeIn}
      />
      <SliderField
        label={t.inspector.audio.fadeOut}
        value={fadeOut.value}
        mixed={fadeOut.mixed}
        disabled={disabled || fadeOutMax <= 0}
        min={0}
        max={Math.max(fadeOutMax, frame)}
        step={frame}
        format={formatFade}
        parse={(text) => parseFade(text, fadeOutMax)}
        unit={t.inspector.units.seconds}
        send={sendFadeOut}
      />
    </>
  );
}
