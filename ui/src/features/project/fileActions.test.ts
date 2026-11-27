import { describe, expect, it, vi } from "vitest";

import { PROJECT_FILE_EXTENSION } from "@/bindings";
import { t } from "@/i18n";

import {
  createFileActions,
  defaultFileName,
  defaultSavePath,
  type FileActionDeps,
  formatWindowTitle,
} from "./fileActions";
import type { UnsavedChoice } from "./unsavedPrompt";

type Fake = FileActionDeps & { calls: string[] };

function fake(options: {
  dirty?: boolean;
  filePath?: string | null;
  choice?: UnsavedChoice;
  saveWrites?: boolean;
  savePath?: string | null;
  openPath?: string | null;
  failing?: string;
  exporting?: boolean;
  cancelExport?: boolean;
}): Fake {
  const calls: string[] = [];
  const step =
    <T>(name: string, value: T) =>
    (...args: unknown[]) => {
      calls.push(args.length ? `${name} ${args.map(String).join(" ")}` : name);
      return options.failing === name ? Promise.reject(new Error(`${name} failed`)) : Promise.resolve(value);
    };
  return {
    calls,
    newProject: step("newProject", null),
    openProject: step("openProject", null),
    saveProject: step("saveProject", options.saveWrites ?? true),
    saveProjectAs: step("saveProjectAs", null),
    confirmClose: step("confirmClose", null),
    current: () => ({ name: "Trip", filePath: options.filePath ?? null, dirty: options.dirty ?? false }),
    askUnsaved: step("askUnsaved", options.choice ?? "cancel"),
    exportRunning: () => options.exporting ?? false,
    askCancelExport: step("askCancelExport", options.cancelExport ?? false),
    stopExport: step("stopExport", undefined),
    pickOpenPath: step("pickOpenPath", options.openPath === undefined ? "C:\\a.cutframe" : options.openPath),
    pickSavePath: step("pickSavePath", options.savePath === undefined ? "C:\\b.cutframe" : options.savePath),
    reportError: (error) => calls.push(`error ${String(error)}`),
    showFailure: (context, error) => calls.push(`${context} failed: ${String(error)}`),
    showView: (view) => calls.push(`show ${view}`),
  };
}

describe("guardUnsaved (through newProject and requestClose)", () => {
  it("runs the action at once when the project is clean", async () => {
    const deps = fake({ dirty: false });
    await createFileActions(deps).newProject();
    expect(deps.calls).toEqual(["newProject", "show editor"]);
  });

  it("Save: saves to the existing file, then runs", async () => {
    const deps = fake({ dirty: true, choice: "save", saveWrites: true });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askUnsaved", "saveProject", "confirmClose"]);
  });

  it("Save without a file: falls back to Save As, then runs", async () => {
    const deps = fake({ dirty: true, choice: "save", saveWrites: false });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual([
      "askUnsaved",
      "saveProject",
      `pickSavePath Trip.${PROJECT_FILE_EXTENSION}`,
      "saveProjectAs C:\\b.cutframe",
      "confirmClose",
    ]);
  });

  it("Save: a cancelled Save As dialog aborts the action", async () => {
    const deps = fake({ dirty: true, choice: "save", saveWrites: false, savePath: null });
    await createFileActions(deps).newProject();
    expect(deps.calls).not.toContain("newProject");
  });

  it("Save: a failed save shows the save error dialog and aborts the action", async () => {
    const deps = fake({ dirty: true, choice: "save", failing: "saveProject" });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askUnsaved", "saveProject", "save failed: Error: saveProject failed"]);
  });

  it("Save As: a failed write shows the save error dialog; a failed picker is a toast", async () => {
    const write = fake({ failing: "saveProjectAs" });
    expect(await createFileActions(write).saveAs()).toBe(false);
    expect(write.calls.at(-1)).toBe("save failed: Error: saveProjectAs failed");

    const picker = fake({ failing: "pickSavePath" });
    expect(await createFileActions(picker).saveAs()).toBe(false);
    expect(picker.calls.at(-1)).toBe(`error Error: pickSavePath failed`);
  });

  it("Don't save: runs the action without saving", async () => {
    const deps = fake({ dirty: true, choice: "discard" });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askUnsaved", "confirmClose"]);
  });

  it("Cancel: does nothing", async () => {
    const deps = fake({ dirty: true, choice: "cancel" });
    await createFileActions(deps).newProject();
    expect(deps.calls).toEqual(["askUnsaved"]);
  });
});

describe("requestClose while exporting", () => {
  it("asks about the export first; Keep exporting stays open and keeps it running", async () => {
    const deps = fake({ exporting: true, cancelExport: false, dirty: true });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askCancelExport"]);
  });

  it("cancels the export, waits for it and closes a clean project", async () => {
    const deps = fake({ exporting: true, cancelExport: true });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askCancelExport", "stopExport", "confirmClose"]);
  });

  it("asks about unsaved changes after the export; Cancel there keeps the export running", async () => {
    const deps = fake({ exporting: true, cancelExport: true, dirty: true, choice: "cancel" });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual(["askCancelExport", "askUnsaved"]);
  });

  it("saves, then cancels the export and closes", async () => {
    const deps = fake({ exporting: true, cancelExport: true, dirty: true, choice: "save" });
    await createFileActions(deps).requestClose();
    expect(deps.calls).toEqual([
      "askCancelExport",
      "askUnsaved",
      "saveProject",
      "stopExport",
      "confirmClose",
    ]);
  });

  it("trusts quitRequested's exporting flag when the UI does not know the export", async () => {
    const deps = fake({ exporting: false, cancelExport: true });
    await createFileActions(deps).requestClose(true);
    expect(deps.calls).toEqual(["askCancelExport", "stopExport", "confirmClose"]);
  });

  it("does not ask without an export", async () => {
    const deps = fake({});
    await createFileActions(deps).requestClose(false);
    expect(deps.calls).toEqual(["confirmClose"]);
  });
});

describe("replaceGuarded", () => {
  it("runs the action after the prompt and resolves true", async () => {
    const deps = fake({ dirty: true, choice: "discard" });
    const ran = await createFileActions(deps).replaceGuarded(() => deps.newProject());
    expect(ran).toBe(true);
    expect(deps.calls).toEqual(["askUnsaved", "newProject"]);
  });

  it("resolves false when cancelled", async () => {
    const deps = fake({ dirty: true, choice: "cancel" });
    await expect(createFileActions(deps).replaceGuarded(() => deps.newProject())).resolves.toBe(false);
    expect(deps.calls).toEqual(["askUnsaved"]);
  });

  it("passes a failure of the action on instead of reporting it", async () => {
    const deps = fake({ failing: "newProject" });
    await expect(createFileActions(deps).replaceGuarded(() => deps.newProject())).rejects.toThrow(
      "newProject failed",
    );
    expect(deps.calls).toEqual(["newProject"]);
  });
});

describe("openProject", () => {
  it("asks first, then opens the picked file", async () => {
    const deps = fake({ dirty: true, choice: "discard" });
    await createFileActions(deps).openProject();
    expect(deps.calls).toEqual(["askUnsaved", "pickOpenPath", "openProject C:\\a.cutframe", "show editor"]);
  });

  it("does nothing when the open dialog is cancelled", async () => {
    const deps = fake({ openPath: null });
    await createFileActions(deps).openProject();
    expect(deps.calls).toEqual(["pickOpenPath"]);
  });

  it("shows a failed open in the open error dialog", async () => {
    const deps = fake({ failing: "openProject" });
    await createFileActions(deps).openProject();
    expect(deps.calls.at(-1)).toBe("open failed: Error: openProject failed");
  });

  it("reports a failed open dialog as a toast", async () => {
    const deps = fake({ failing: "pickOpenPath" });
    await createFileActions(deps).openProject();
    expect(deps.calls.at(-1)).toBe("error Error: pickOpenPath failed");
  });
});

describe("openPath", () => {
  it("opens the path behind the unsaved-changes prompt", async () => {
    const deps = fake({ dirty: true, choice: "discard" });
    await createFileActions(deps).openPath("D:\\v\\a.cutframe");
    expect(deps.calls).toEqual(["askUnsaved", "openProject D:\\v\\a.cutframe", "show editor"]);
  });

  it("does nothing when the prompt is cancelled", async () => {
    const deps = fake({ dirty: true, choice: "cancel" });
    await createFileActions(deps).openPath("D:\\a.cutframe");
    expect(deps.calls).toEqual(["askUnsaved"]);
  });

  it("shows a failed open like File → Open", async () => {
    const deps = fake({ failing: "openProject" });
    await createFileActions(deps).openPath("D:\\missing.cutframe");
    expect(deps.calls.at(-1)).toBe("open failed: Error: openProject failed");
  });

  it("is ignored while another file action is running", async () => {
    const deps = fake({ dirty: true });
    let answer: (choice: UnsavedChoice) => void = () => undefined;
    deps.askUnsaved = vi.fn(() => new Promise<UnsavedChoice>((resolve) => (answer = resolve)));
    const actions = createFileActions(deps);
    const first = actions.newProject();
    await actions.openPath("D:\\a.cutframe");
    answer("discard");
    await first;
    expect(deps.calls).toEqual(["newProject", "show editor"]);
  });
});

describe("goHome", () => {
  it("replaces the project with a new one, then shows the home screen", async () => {
    const deps = fake({ dirty: true, choice: "discard" });
    await createFileActions(deps).goHome();
    expect(deps.calls).toEqual(["askUnsaved", "newProject", "show home"]);
  });

  it("Cancel at the unsaved prompt stays in the editor", async () => {
    const deps = fake({ dirty: true, choice: "cancel" });
    await createFileActions(deps).goHome();
    expect(deps.calls).toEqual(["askUnsaved"]);
  });

  it("a failed new project reports the error and stays in the editor", async () => {
    const deps = fake({ failing: "newProject" });
    await createFileActions(deps).goHome();
    expect(deps.calls).toEqual(["newProject", "error Error: newProject failed"]);
  });
});

describe("openPath and openProject failures", () => {
  it("do not leave the current view", async () => {
    const deps = fake({ failing: "openProject" });
    const actions = createFileActions(deps);
    await actions.openPath("D:\\gone.cutframe");
    await actions.openProject();
    expect(deps.calls.some((call) => call.startsWith("show"))).toBe(false);
  });
});

describe("save and saveAs", () => {
  it("save resolves true when written and does not open a dialog", async () => {
    const deps = fake({ saveWrites: true });
    await expect(createFileActions(deps).save()).resolves.toBe(true);
    expect(deps.calls).toEqual(["saveProject"]);
  });

  it("saveAs proposes the name in the current file's folder", async () => {
    const deps = fake({ filePath: "D:\\videos\\old.cutframe" });
    await expect(createFileActions(deps).saveAs()).resolves.toBe(true);
    expect(deps.calls[0]).toBe(`pickSavePath D:\\videos\\Trip.${PROJECT_FILE_EXTENSION}`);
  });

  it("ignores a second action while one is running", async () => {
    const deps = fake({ dirty: true });
    let answer: (choice: UnsavedChoice) => void = () => undefined;
    deps.askUnsaved = vi.fn(() => new Promise<UnsavedChoice>((resolve) => (answer = resolve)));
    const actions = createFileActions(deps);
    const first = actions.newProject();
    await expect(actions.save()).resolves.toBe(false);
    answer("discard");
    await first;
    expect(deps.calls).toEqual(["newProject", "show editor"]);
    expect(deps.askUnsaved).toHaveBeenCalledTimes(1);
  });
});

describe("file names and titles", () => {
  it("replaces characters Windows forbids", () => {
    expect(defaultFileName('a<b>c:d"e/f\\g|h?i*j')).toBe(`a_b_c_d_e_f_g_h_i_j.${PROJECT_FILE_EXTENSION}`);
    expect(defaultFileName("Trip. ")).toBe(`Trip.${PROJECT_FILE_EXTENSION}`);
    expect(defaultFileName("   ")).toBe(`${t.file.untitled}.${PROJECT_FILE_EXTENSION}`);
  });

  it("keeps the folder of the current file", () => {
    expect(defaultSavePath("New", null)).toBe(`New.${PROJECT_FILE_EXTENSION}`);
    expect(defaultSavePath("New", "C:\\x\\y.cutframe")).toBe(`C:\\x\\New.${PROJECT_FILE_EXTENSION}`);
  });

  it("marks a dirty project with an asterisk", () => {
    expect(formatWindowTitle({ name: "Trip", dirty: true })).toBe("*Trip - Cutframe");
    expect(formatWindowTitle({ name: "Trip", dirty: false })).toBe("Trip - Cutframe");
    expect(formatWindowTitle(null)).toBe(t.app.name);
  });
});
