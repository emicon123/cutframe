import { describe, expect, it } from "vitest";

import { createJobResults } from "./jobResults";

const files = [{ path: "C:\\a.mp4", outcome: { kind: "relinked" as const, media: "m1" } }];

describe("jobResults", () => {
  it("resolves a waiter with the report and result", async () => {
    const results = createJobResults();
    const outcome = results.wait("j");
    results.report({ jobId: "j", files });
    results.finished({ jobId: "j", result: { Ok: null } });
    await expect(outcome).resolves.toEqual({ files, error: null });
  });

  it("remembers a job that finished before anyone waited", async () => {
    const results = createJobResults();
    results.report({ jobId: "j", files });
    results.finished({ jobId: "j", result: { Err: { kind: "io", message: "x" } } });
    await expect(results.wait("j")).resolves.toEqual({ files, error: { kind: "io", message: "x" } });
  });

  it("gives an empty file list without a report", async () => {
    const results = createJobResults();
    results.finished({ jobId: "j", result: { Ok: null } });
    await expect(results.wait("j")).resolves.toEqual({ files: [], error: null });
  });
});
