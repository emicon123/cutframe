import { describe, expect, it } from "vitest";

import type { HistoryLabel, IpcError, JobStage, MediaErrorKind } from "@/bindings";

import { errorKindText, historyLabelText, jobStageText, mediaErrorText, t } from "./index";

describe("historyLabelText", () => {
  it.each<[HistoryLabel, string]>([
    ["addTrack", "Add Track"],
    ["removeTrack", "Remove Track"],
    ["renameTrack", "Rename Track"],
    ["renameProject", "Rename Project"],
    ["setProjectSettings", "Change Project Settings"],
    ["importMedia", "Import Media"],
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
    "cancelled",
  ])("translates %s", (kind) => {
    expect(errorKindText(kind)).toBe(t.errors.kinds[kind]);
    expect(errorKindText(kind)).not.toBe(kind);
  });

  it("falls back to generic text for an unknown kind", () => {
    expect(errorKindText("diskFull")).toBe(t.errors.unknown);
    expect(errorKindText("constructor")).toBe(t.errors.unknown);
  });
});

describe("mediaErrorText", () => {
  it.each<MediaErrorKind>([
    "fileNotFound",
    "accessDenied",
    "unsupportedPath",
    "unsupportedFormat",
    "unsupportedCodec",
    "damaged",
    "tooManyFiles",
  ])("translates %s", (kind) => {
    expect(mediaErrorText(kind)).toBe(t.mediaBin.errors[kind]);
    expect(mediaErrorText(kind)).not.toBe(kind);
  });

  it("falls back to generic text for an unknown kind", () => {
    expect(mediaErrorText("kindMismatch")).toBe(t.mediaBin.unknownError);
    expect(mediaErrorText("hasOwnProperty")).toBe(t.mediaBin.unknownError);
  });
});

describe("jobStageText", () => {
  it.each<JobStage>(["probing"])("translates %s", (stage) => {
    expect(jobStageText(stage)).toBe(t.mediaBin.job.stages[stage]);
  });

  it("falls back to generic text for an unknown stage", () => {
    expect(jobStageText("generatingPreviews")).toBe(t.mediaBin.job.unknownStage);
  });
});
