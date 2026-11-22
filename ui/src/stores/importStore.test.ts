import { describe, expect, it } from "vitest";

import type { MediaFileResult } from "@/bindings";

import {
  type ImportAction,
  importReducer,
  type ImportState,
  initialImportState,
  isBackgroundJob,
  isCleanSummary,
  isImportJob,
  summarizeReport,
} from "./importStore";

const files: MediaFileResult[] = [
  { path: "C:\\clips\\a.mp4", outcome: { kind: "added", media: "m1" } },
  { path: "C:\\clips\\b.mp4", outcome: { kind: "alreadyImported", media: "m0" } },
  { path: "C:\\clips\\c.txt", outcome: { kind: "failed", error: "unsupportedFormat", detail: "no demuxer" } },
  { path: "C:\\clips\\d.mov", outcome: { kind: "added", media: "m2" } },
  { path: "C:\\clips\\e.mp4", outcome: { kind: "failed", error: "damaged", detail: "truncated" } },
];

const ok = { Ok: null };
const cancelled = { Err: { kind: "cancelled" as const, message: "cancelled" } };

function reduce(actions: ImportAction[], state: ImportState = initialImportState): ImportState {
  return actions.reduce(importReducer, state);
}

describe("summarizeReport", () => {
  it("counts outcomes and keeps failures in input order", () => {
    expect(summarizeReport("j", files)).toEqual({
      jobId: "j",
      added: 2,
      alreadyImported: 1,
      failed: [
        { path: "C:\\clips\\c.txt", error: "unsupportedFormat" },
        { path: "C:\\clips\\e.mp4", error: "damaged" },
      ],
      jobError: null,
    });
  });

  it("is clean only when every file was added", () => {
    expect(isCleanSummary(summarizeReport("j", files))).toBe(false);
    expect(isCleanSummary(summarizeReport("j", files.slice(0, 1)))).toBe(true);
    expect(isCleanSummary(summarizeReport("j", files.slice(1, 2)))).toBe(false);
  });
});

describe("importReducer", () => {
  it("tracks a started job and its progress", () => {
    const state = reduce([
      { type: "started", id: "j" },
      { type: "progress", event: { jobId: "j", stage: "probing", progress: null } },
      { type: "progress", event: { jobId: "j", stage: "probing", progress: 0.5 } },
    ]);
    expect(state.jobs).toEqual([
      { id: "j", started: true, stage: "probing", progress: 0.5, cancelling: false, files: null },
    ]);
  });

  it("summarises the report when the job finishes", () => {
    const state = reduce([
      { type: "started", id: "j" },
      { type: "report", event: { jobId: "j", files } },
      { type: "finished", event: { jobId: "j", result: ok } },
    ]);
    expect(state.jobs).toEqual([]);
    expect(state.summary).toEqual(summarizeReport("j", files));
  });

  it("handles events that arrive before the command resolves", () => {
    const state = reduce([
      { type: "progress", event: { jobId: "j", stage: "probing", progress: 1 } },
      { type: "report", event: { jobId: "j", files } },
      { type: "finished", event: { jobId: "j", result: ok } },
      { type: "started", id: "j" },
      { type: "progress", event: { jobId: "j", stage: "probing", progress: 1 } },
    ]);
    expect(state.jobs).toEqual([]);
    expect(state.summary?.added).toBe(2);
  });

  it("marks a cancel request and shows no result for a cancelled job", () => {
    const requested = reduce([
      { type: "started", id: "j" },
      { type: "cancelRequested", id: "j" },
    ]);
    expect(requested.jobs[0]?.cancelling).toBe(true);
    const done = reduce([{ type: "finished", event: { jobId: "j", result: cancelled } }], requested);
    expect(done.jobs).toEqual([]);
    expect(done.summary).toBeNull();
  });

  it("ignores a cancel request for an unknown job", () => {
    expect(reduce([{ type: "cancelRequested", id: "x" }])).toBe(initialImportState);
  });

  it("reports a failed job by its error kind", () => {
    const state = reduce([
      { type: "started", id: "j" },
      { type: "finished", event: { jobId: "j", result: { Err: { kind: "internal", message: "boom" } } } },
    ]);
    expect(state.summary).toEqual({
      jobId: "j",
      added: 0,
      alreadyImported: 0,
      failed: [],
      jobError: "internal",
    });
  });

  it("keeps other jobs running and ignores a repeated finish", () => {
    const state = reduce([
      { type: "started", id: "a" },
      { type: "started", id: "b" },
      { type: "report", event: { jobId: "a", files } },
      { type: "finished", event: { jobId: "a", result: ok } },
      { type: "finished", event: { jobId: "a", result: cancelled } },
    ]);
    expect(state.jobs.map((j) => j.id)).toEqual(["b"]);
    expect(state.summary?.jobId).toBe("a");
  });

  it("treats preview generation as background work without a result", () => {
    const progress = (stage: "generatingPreviews" | "probing") => ({
      type: "progress" as const,
      event: { jobId: "bg", stage, progress: 0.5 },
    });
    const running = reduce([progress("generatingPreviews")]);
    expect(running.jobs.map(isBackgroundJob)).toEqual([true]);
    expect(isBackgroundJob({ stage: "probing" })).toBe(false);
    expect(isBackgroundJob({ stage: null })).toBe(false);

    const failed = reduce([
      progress("generatingPreviews"),
      { type: "finished", event: { jobId: "bg", result: { Err: { kind: "internal", message: "" } } } },
    ]);
    expect(failed.jobs).toEqual([]);
    expect(failed.summary).toBeNull();
  });

  it("remembers a bounded number of finished ids", () => {
    const actions: ImportAction[] = Array.from({ length: 100 }, (_, i) => ({
      type: "finished",
      event: { jobId: `j${i}`, result: cancelled },
    }));
    expect(reduce(actions).finished.length).toBeLessThanOrEqual(32);
  });

  it("never summarises a job nobody started as an import (relink)", () => {
    const state = reduce([
      { type: "progress", event: { jobId: "r", stage: "probing", progress: null } },
      { type: "report", event: { jobId: "r", files } },
      { type: "finished", event: { jobId: "r", result: ok } },
    ]);
    expect(state.jobs).toEqual([]);
    expect(state.summary).toBeNull();
    expect(isImportJob({ started: false, stage: "probing" })).toBe(false);
    expect(isImportJob({ started: true, stage: "probing" })).toBe(true);
    expect(isImportJob({ started: true, stage: "checkingMedia" })).toBe(false);
  });

  it("dismisses the summary", () => {
    const state = reduce([
      { type: "started", id: "j" },
      { type: "finished", event: { jobId: "j", result: { Err: { kind: "internal", message: "" } } } },
      { type: "dismissSummary" },
    ]);
    expect(state.summary).toBeNull();
  });
});
