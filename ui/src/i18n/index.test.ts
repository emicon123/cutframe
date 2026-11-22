import { describe, expect, it } from "vitest";

import type { HistoryLabel, IpcError } from "@/bindings";

import { errorKindText, historyLabelText, t } from "./index";

describe("historyLabelText", () => {
  it.each<[HistoryLabel, string]>([
    ["addTrack", "Add Track"],
    ["removeTrack", "Remove Track"],
    ["renameTrack", "Rename Track"],
    ["renameProject", "Rename Project"],
    ["setProjectSettings", "Change Project Settings"],
  ])("translates %s", (label, text) => {
    expect(historyLabelText(label)).toBe(text);
    expect(t.edit.undoAction(historyLabelText(label))).toBe(`Undo ${text}`);
  });

  it("falls back to generic text for an unknown key, never the raw key", () => {
    expect(historyLabelText("moveClip")).toBe(t.history.unknown);
    expect(historyLabelText("toString")).toBe(t.history.unknown);
  });
});

describe("errorKindText", () => {
  it.each<IpcError["kind"]>([
    "invalidEdit",
    "unavailable",
    "internal",
    "fileNotFound",
    "malformedProject",
    "projectTooNew",
    "invalidProject",
    "io",
  ])("translates %s", (kind) => {
    expect(errorKindText(kind)).toBe(t.errors.kinds[kind]);
    expect(errorKindText(kind)).not.toBe(kind);
  });

  it("falls back to generic text for an unknown kind", () => {
    expect(errorKindText("diskFull")).toBe(t.errors.unknown);
    expect(errorKindText("constructor")).toBe(t.errors.unknown);
  });
});
