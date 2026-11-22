import { describe, expect, it } from "vitest";

import type { IpcError } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  AUTOSAVE_RANGE,
  errorField,
  flicksToSecondsText,
  formatBytes,
  formFromPreferences,
  IMAGE_DURATION_RANGE,
  parseSeconds,
  type Preferences,
  preferencesFromForm,
  secondsErrorText,
  secondsToFlicks,
  TRANSITION_DURATION_RANGE,
  validateForm,
} from "./preferences";

const prefs = (overrides: Partial<Preferences> = {}): Preferences => ({
  schemaVersion: 1,
  autosaveInterval: 60 * FLICKS_PER_SECOND,
  defaultImageDuration: 5 * FLICKS_PER_SECOND,
  defaultTransitionDuration: FLICKS_PER_SECOND / 2,
  language: "en",
  cacheDir: "D:\\Cache",
  ...overrides,
});

describe("seconds ↔ flicks", () => {
  it("round-trips the values the fields show", () => {
    for (const text of ["0.1", "0.5", "5", "15", "600", "3600", "2.3"]) {
      expect(flicksToSecondsText(secondsToFlicks(Number(text)))).toBe(text);
    }
  });

  it("gives integer flicks", () => {
    expect(secondsToFlicks(0.1)).toBe(70_560_000);
    expect(Number.isInteger(secondsToFlicks(1 / 3))).toBe(true);
  });
});

describe("parseSeconds", () => {
  it("accepts the range bounds and rejects values just outside", () => {
    expect(parseSeconds("15", AUTOSAVE_RANGE)).toEqual({ ok: true, flicks: 15 * FLICKS_PER_SECOND });
    expect(parseSeconds("3600", AUTOSAVE_RANGE).ok).toBe(true);
    expect(parseSeconds("14", AUTOSAVE_RANGE)).toEqual({ ok: false, error: "outOfRange" });
    expect(parseSeconds("3601", AUTOSAVE_RANGE)).toEqual({ ok: false, error: "outOfRange" });
    expect(parseSeconds("0.1", IMAGE_DURATION_RANGE).ok).toBe(true);
    expect(parseSeconds("0.09", IMAGE_DURATION_RANGE).ok).toBe(false);
    expect(parseSeconds("600", IMAGE_DURATION_RANGE).ok).toBe(true);
    expect(parseSeconds("10", TRANSITION_DURATION_RANGE).ok).toBe(true);
    expect(parseSeconds("10.1", TRANSITION_DURATION_RANGE).ok).toBe(false);
  });

  it("requires whole seconds for autosave", () => {
    expect(parseSeconds("30.5", AUTOSAVE_RANGE)).toEqual({ ok: false, error: "notWhole" });
  });

  it("rejects empty and non-numeric text", () => {
    expect(parseSeconds("", IMAGE_DURATION_RANGE)).toEqual({ ok: false, error: "notNumber" });
    expect(parseSeconds("  ", IMAGE_DURATION_RANGE)).toEqual({ ok: false, error: "notNumber" });
    expect(parseSeconds("abc", IMAGE_DURATION_RANGE)).toEqual({ ok: false, error: "notNumber" });
    expect(parseSeconds("Infinity", IMAGE_DURATION_RANGE)).toEqual({ ok: false, error: "notNumber" });
  });

  it("translates the errors", () => {
    expect(secondsErrorText("outOfRange", TRANSITION_DURATION_RANGE)).toBe(
      t.preferences.fieldErrors.outOfRange("0.1", "10"),
    );
    expect(secondsErrorText("notWhole", AUTOSAVE_RANGE)).toBe(t.preferences.fieldErrors.notWhole);
  });
});

describe("form", () => {
  it("shows the stored values in seconds", () => {
    expect(formFromPreferences(prefs())).toEqual({
      language: "en",
      autosaveEnabled: true,
      autosaveSeconds: "60",
      imageSeconds: "5",
      transitionSeconds: "0.5",
    });
  });

  it("offers the default interval when autosave is off", () => {
    const form = formFromPreferences(prefs({ autosaveInterval: null }));
    expect(form.autosaveEnabled).toBe(false);
    expect(form.autosaveSeconds).toBe("60");
  });

  it("ignores the interval field while autosave is off", () => {
    const form = { ...formFromPreferences(prefs()), autosaveEnabled: false, autosaveSeconds: "x" };
    expect(validateForm(form)).toEqual({});
  });

  it("lists every invalid field", () => {
    const form = {
      ...formFromPreferences(prefs()),
      autosaveSeconds: "5",
      imageSeconds: "",
      transitionSeconds: "11",
    };
    expect(validateForm(form)).toEqual({
      autosaveSeconds: "outOfRange",
      imageSeconds: "notNumber",
      transitionSeconds: "outOfRange",
    });
  });
});

describe("preferencesFromForm", () => {
  it("sends the full object with the edited values in flicks", () => {
    const base = prefs();
    const initial = formFromPreferences(base);
    const result = preferencesFromForm(
      { ...initial, autosaveSeconds: "120", imageSeconds: "2.5", transitionSeconds: "1" },
      base,
      initial,
    );
    expect(result).toEqual({
      schemaVersion: 1,
      autosaveInterval: 120 * FLICKS_PER_SECOND,
      defaultImageDuration: 2.5 * FLICKS_PER_SECOND,
      defaultTransitionDuration: FLICKS_PER_SECOND,
      language: "en",
      cacheDir: "D:\\Cache",
    });
  });

  it("sends an explicit null when autosave is switched off", () => {
    const base = prefs();
    const initial = formFromPreferences(base);
    const result = preferencesFromForm({ ...initial, autosaveEnabled: false }, base, initial);
    expect(result).not.toBeNull();
    expect(result).toHaveProperty("autosaveInterval", null);
  });

  it("keeps the exact flicks of fields that were not edited", () => {
    const odd = 5 * FLICKS_PER_SECOND + 1;
    const base = prefs({ defaultImageDuration: odd, autosaveInterval: null });
    const initial = formFromPreferences(base);
    const result = preferencesFromForm({ ...initial, autosaveEnabled: true }, base, initial);
    expect(result?.defaultImageDuration).toBe(odd);
    expect(result?.autosaveInterval).toBe(60 * FLICKS_PER_SECOND);
  });

  it("gives null for an invalid form", () => {
    const base = prefs();
    const initial = formFromPreferences(base);
    expect(preferencesFromForm({ ...initial, imageSeconds: "0" }, base, initial)).toBeNull();
  });
});

describe("errorField", () => {
  const invalid = (message: string): IpcError => ({ kind: "invalidEdit", message });

  it("maps Rust's field names to form fields", () => {
    expect(errorField(invalid("defaultImageDuration is out of range"))).toBe("imageSeconds");
    expect(errorField(invalid("defaultTransitionDuration is out of range"))).toBe("transitionSeconds");
    expect(errorField(invalid("autosaveInterval must be a whole number of seconds"))).toBe("autosaveSeconds");
  });

  it("gives null for other fields, kinds and values", () => {
    expect(errorField(invalid("schemaVersion 2 is not the current version 1"))).toBeNull();
    expect(errorField(invalid("cacheDir must be an absolute path"))).toBeNull();
    expect(errorField(invalid("constructor"))).toBeNull();
    expect(errorField({ kind: "io", message: "defaultImageDuration" })).toBeNull();
    expect(errorField(new Error("x"))).toBeNull();
    expect(errorField(null)).toBeNull();
  });
});

describe("formatBytes", () => {
  it("shows small sizes in bytes", () => {
    expect(formatBytes("0")).toBe(t.units.bytes("0"));
    expect(formatBytes("1023")).toBe(t.units.bytes("1023"));
  });

  it("scales by 1024 and truncates to one decimal", () => {
    expect(formatBytes("1024")).toBe(t.units.scaled("1.0", "KB"));
    expect(formatBytes("1536")).toBe(t.units.scaled("1.5", "KB"));
    expect(formatBytes(String(1024 * 1024 - 1))).toBe(t.units.scaled("1023.9", "KB"));
    expect(formatBytes(String(3 * 1024 ** 3))).toBe(t.units.scaled("3.0", "GB"));
  });

  it("stays exact above 2^53", () => {
    // 2^60 + 1 bytes is 1048576 TB; a float would round the byte count, BigInt does not.
    expect(formatBytes("1152921504606846977")).toBe(t.units.scaled("1048576.0", "TB"));
    expect(formatBytes("9007199254740993")).toBe(t.units.scaled("8192.0", "TB"));
  });

  it("does not throw on a malformed size", () => {
    expect(formatBytes("")).toBe(t.preferences.cache.sizeUnknown);
    expect(formatBytes("12.5")).toBe(t.preferences.cache.sizeUnknown);
    expect(formatBytes("-1")).toBe(t.preferences.cache.sizeUnknown);
  });
});
