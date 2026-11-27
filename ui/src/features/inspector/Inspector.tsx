/**
 * The inspector panel (clip-speed-audio design §6): project info with no clip selected, otherwise
 * the clip sections the selection has in common (`SECTIONS`), each editing every selected clip it
 * applies to. Values come from the model; mixed values show as such.
 */
import { Fragment, type ReactNode, useState } from "react";
import { useStore } from "zustand";

import { commands, type ProjectView_Serialize } from "@/bindings";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { t } from "@/i18n";
import { formatFrameRate, formatTimecode } from "@/lib/time";
import { run } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";
import { selectionStore } from "@/stores/selectionStore";

import { SECTIONS } from "./registry";
import { anyLocked, type InspectorItem, inspectorItems, visibleSections } from "./sections";

export function Inspector() {
  const project = useProjectStore((s) => s.state?.project);
  const selected = useStore(selectionStore, (s) => s.clips);
  const items = project ? inspectorItems(project, selected) : [];
  const title = items.length > 0 ? t.inspector.clip(items.length) : t.inspector.title;

  return (
    <section className="flex h-full flex-col bg-panel" aria-label={title}>
      <header className="flex h-9 shrink-0 items-center border-b px-3">
        <h2 className="font-semibold">{title}</h2>
      </header>
      {project &&
        (items.length > 0 ? (
          <ClipInspector items={items} project={project} />
        ) : (
          <ProjectInfo project={project} />
        ))}
    </section>
  );
}

function ClipInspector({ items, project }: { items: InspectorItem[]; project: ProjectView_Serialize }) {
  const sections = visibleSections(SECTIONS, items);
  if (sections.length === 0) return <p className="p-3 text-muted-foreground">{t.inspector.noCommon}</p>;
  return (
    <div className="flex flex-col overflow-y-auto">
      {anyLocked(items) && <p className="px-3 pt-3 text-xs text-muted-foreground">{t.inspector.locked}</p>}
      {sections.map(({ section, targets }, index) => (
        <Fragment key={section.id}>
          {index > 0 && <Separator />}
          <section aria-label={section.title} className="flex flex-col gap-3 p-3">
            <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
              {section.title}
            </h3>
            {/* Remount for another selection, so a half-typed value never jumps to other clips. */}
            <section.Component
              key={targets.map((item) => item.clip.id).join()}
              targets={targets}
              project={project}
            />
          </section>
        </Fragment>
      ))}
    </div>
  );
}

function ProjectInfo({ project }: { project: ProjectView_Serialize }) {
  return (
    <div className="flex flex-col gap-4 overflow-y-auto p-3">
      {/* Remount on outside changes (undo/redo) so the field shows the model's name. */}
      <ProjectNameField key={project.name} name={project.name} />
      <Separator />
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2">
        <Row label={t.inspector.resolution}>
          {project.settings.width} × {project.settings.height}
        </Row>
        <Row label={t.inspector.frameRate}>{formatFrameRate(project.settings.frameRate)}</Row>
        <Row label={t.inspector.sampleRate}>{(project.settings.sampleRate / 1000).toFixed(1)} kHz</Row>
        <Row label={t.inspector.duration}>{formatTimecode(project.duration, project.settings.frameRate)}</Row>
        <Row label={t.inspector.tracks}>{project.tracks.length}</Row>
      </dl>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right tabular-nums">{children}</dd>
    </>
  );
}

function ProjectNameField({ name }: { name: ProjectView_Serialize["name"] }) {
  const [draft, setDraft] = useState(name);
  const commit = () => {
    const next = draft.trim();
    if (next && next !== name) void run(commands.renameProject(next));
    else setDraft(name);
  };
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs text-muted-foreground">{t.inspector.name}</span>
      <Input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            setDraft(name);
            e.currentTarget.blur();
          }
        }}
      />
    </label>
  );
}
