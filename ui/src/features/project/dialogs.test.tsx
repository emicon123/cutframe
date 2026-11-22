import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { mockCommands, type ProjectSettings } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";

import { ProjectSettingsDialog } from "./ProjectSettingsDialog";
import { UnsavedChangesDialog } from "./UnsavedChangesDialog";
import { askUnsaved } from "./unsavedPrompt";

const state: ProjectState = {
  revision: 1,
  project: {
    id: "p",
    name: "Trip",
    settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
    duration: 0,
    tracks: [],
    media: [],
  },
  history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null, entries: [], current: 0 },
  filePath: null,
  dirty: true,
};

afterEach(() => {
  useProjectStore.setState({ state: null });
  useAppStore.setState({ previewOccluders: 0 });
});

describe("UnsavedChangesDialog", () => {
  it("answers the prompt and hides the preview while open", async () => {
    useProjectStore.setState({ state });
    render(<UnsavedChangesDialog />);
    const answer = askUnsaved();

    fireEvent.click(await screen.findByRole("button", { name: t.unsaved.discard }));
    await expect(answer).resolves.toBe("discard");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useAppStore.getState().previewOccluders).toBe(0);
  });

  it("treats Escape as Cancel", async () => {
    useProjectStore.setState({ state });
    render(<UnsavedChangesDialog />);
    const answer = askUnsaved();
    await screen.findByRole("dialog");
    expect(useAppStore.getState().previewOccluders).toBe(1);

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await expect(answer).resolves.toBe("cancel");
  });
});

describe("ProjectSettingsDialog", () => {
  it("applies a preset as one setProjectSettings call", async () => {
    const sent: ProjectSettings[] = [];
    mockCommands({
      setProjectSettings: ({ settings }) => {
        sent.push(settings);
        return null;
      },
    });
    useProjectStore.setState({ state });
    let open = true;
    render(<ProjectSettingsDialog open onOpenChange={(next) => (open = next)} />);

    fireEvent.change(screen.getByLabelText(t.projectSettings.resolution), { target: { value: "shorts" } });
    expect(screen.getByText("9:16")).toBeDefined();
    fireEvent.change(screen.getByLabelText(t.projectSettings.frameRate), { target: { value: "30000/1001" } });
    fireEvent.click(screen.getByRole("button", { name: t.projectSettings.apply }));

    await waitFor(() => expect(open).toBe(false));
    expect(sent).toEqual([
      { width: 1080, height: 1920, frameRate: { num: 30000, den: 1001 }, sampleRate: 48000 },
    ]);
  });

  it("blocks Apply for an odd custom size", () => {
    useProjectStore.setState({ state });
    render(<ProjectSettingsDialog open onOpenChange={() => undefined} />);

    fireEvent.change(screen.getByLabelText(t.projectSettings.resolution), { target: { value: "custom" } });
    fireEvent.change(screen.getByLabelText(t.projectSettings.width), { target: { value: "1921" } });
    expect(screen.getByText(t.projectSettings.sizeErrors.odd)).toBeDefined();
    expect(screen.getByRole("button", { name: t.projectSettings.apply })).toHaveProperty("disabled", true);
  });
});
