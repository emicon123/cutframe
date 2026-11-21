import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND, formatFrameRate, formatTimecode, frameDuration } from "./time";

const fps30 = { num: 30, den: 1 };
const ntsc = { num: 30000, den: 1001 };

describe("frameDuration", () => {
  it("is an exact integer for NTSC rates", () => {
    expect(Number.isInteger(frameDuration(ntsc))).toBe(true);
    expect(frameDuration(fps30)).toBe(FLICKS_PER_SECOND / 30);
  });
});

describe("formatTimecode", () => {
  it("formats zero and negative times", () => {
    expect(formatTimecode(0, fps30)).toBe("00:00:00:00");
    expect(formatTimecode(-5, fps30)).toBe("00:00:00:00");
  });

  it("formats hours, minutes, seconds and frames", () => {
    const t = (3600 + 2 * 60 + 3) * FLICKS_PER_SECOND + 4 * frameDuration(fps30);
    expect(formatTimecode(t, fps30)).toBe("01:02:03:04");
  });

  it("floors to the frame containing the time", () => {
    expect(formatTimecode(frameDuration(fps30) - 1, fps30)).toBe("00:00:00:00");
  });

  it("counts NTSC frames non-drop", () => {
    expect(formatTimecode(30 * frameDuration(ntsc), ntsc)).toBe("00:00:01:00");
  });
});

describe("formatFrameRate", () => {
  it("shows integer and fractional rates", () => {
    expect(formatFrameRate(fps30)).toBe("30 fps");
    expect(formatFrameRate(ntsc)).toBe("29.970 fps");
  });
});
