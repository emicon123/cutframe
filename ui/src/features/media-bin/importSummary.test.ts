import { describe, expect, it } from "vitest";

import { t } from "@/i18n";

import { summaryLines } from "./importSummary";

const base = { jobId: "j", added: 0, alreadyImported: 0, failed: [], jobError: null };

describe("summaryLines", () => {
  it("lists counts that are not zero, with singular and plural", () => {
    expect(summaryLines({ ...base, added: 1 })).toEqual(["1 file imported"]);
    expect(
      summaryLines({
        ...base,
        added: 3,
        alreadyImported: 2,
        failed: [{ path: "a", error: "damaged" }],
      }),
    ).toEqual(["3 files imported", "2 files already imported", "1 file could not be imported"]);
  });

  it("says when nothing was found", () => {
    expect(summaryLines(base)).toEqual([t.mediaBin.result.nothing]);
  });

  it("translates a job failure by kind", () => {
    expect(summaryLines({ ...base, jobError: "internal" })).toEqual(["Import failed: Something went wrong"]);
  });
});
