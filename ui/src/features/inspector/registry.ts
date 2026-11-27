/**
 * The inspector's clip sections, in display order. Adding a section (task 0350's Transform:
 * `applies: (item) => item.clip.visual != null`, listed first) is one entry here; the shell picks
 * the sections the selection has in common (`visibleSections`).
 */
import { t } from "@/i18n";

import { AudioSection } from "./AudioSection";
import { DurationSection } from "./DurationSection";
import { hasAudio, hasStillDuration, type InspectorSection, isRetimableItem } from "./sections";
import { SpeedSection } from "./SpeedSection";

export const SECTIONS: readonly InspectorSection[] = [
  { id: "audio", title: t.inspector.audio.title, applies: hasAudio, Component: AudioSection },
  { id: "speed", title: t.inspector.speed.title, applies: isRetimableItem, Component: SpeedSection },
  {
    id: "duration",
    title: t.inspector.still.title,
    applies: hasStillDuration,
    single: true,
    Component: DurationSection,
  },
];
