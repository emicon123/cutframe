import { describe, expect, it } from "vitest";

import {
  filterPackages,
  isSupported,
  loadLicenses,
  packageKey,
  packageLink,
  pageOf,
  textsOf,
  type ThirdPartyLicenses,
  type ThirdPartyPackage,
} from "./licenses";

const pkg = (
  name: string,
  license: string,
  ecosystem: ThirdPartyPackage["ecosystem"] = "cargo",
  version = "1.0.0",
): ThirdPartyPackage => ({
  ecosystem,
  name,
  version,
  license,
  repository: null,
  homepage: null,
  authors: [],
  licenseTextIds: [],
});

const packages = [
  pkg("serde", "MIT OR Apache-2.0"),
  pkg("react", "MIT", "npm", "19.3.1"),
  pkg("wgpu", "MIT OR Apache-2.0", "cargo", "26.0.0"),
  pkg("Rust standard library", "MIT OR Apache-2.0", "rust", "1.100.0"),
];

describe("filterPackages", () => {
  it("returns everything for an empty or blank query", () => {
    expect(filterPackages(packages, "")).toEqual(packages);
    expect(filterPackages(packages, "   ")).toEqual(packages);
  });

  it("matches name, licence, version and ecosystem case-insensitively", () => {
    expect(filterPackages(packages, "REACT").map((p) => p.name)).toEqual(["react"]);
    expect(filterPackages(packages, "apache").map((p) => p.name)).toEqual([
      "serde",
      "wgpu",
      "Rust standard library",
    ]);
    expect(filterPackages(packages, "19.3").map((p) => p.name)).toEqual(["react"]);
    expect(filterPackages(packages, "npm").map((p) => p.name)).toEqual(["react"]);
  });

  it("needs every word to match", () => {
    expect(filterPackages(packages, "mit wgpu").map((p) => p.name)).toEqual(["wgpu"]);
    expect(filterPackages(packages, "react apache")).toEqual([]);
  });
});

describe("pageOf", () => {
  it("returns the first rows and how many remain", () => {
    expect(pageOf([1, 2, 3, 4, 5], 2)).toEqual({ visible: [1, 2], remaining: 3 });
    expect(pageOf([1, 2], 50)).toEqual({ visible: [1, 2], remaining: 0 });
    expect(pageOf([1, 2], -1)).toEqual({ visible: [], remaining: 2 });
  });
});

describe("textsOf", () => {
  const data = {
    licenseTexts: { a: { name: "LICENSE-MIT", text: "MIT text" } },
  } as unknown as ThirdPartyLicenses;

  it("resolves ids and skips unknown ones and prototype names", () => {
    expect(textsOf(data, ["a", "missing", "toString"])).toEqual([
      { id: "a", name: "LICENSE-MIT", text: "MIT text" },
    ]);
  });
});

describe("packageKey and packageLink", () => {
  it("tells two versions of one crate apart", () => {
    expect(packageKey(pkg("syn", "MIT", "cargo", "1.0.0"))).not.toBe(
      packageKey(pkg("syn", "MIT", "cargo", "2.0.0")),
    );
  });

  it("prefers the repository over the homepage", () => {
    expect(packageLink({ ...pkg("a", "MIT"), repository: "https://r", homepage: "https://h" })).toBe(
      "https://r",
    );
    expect(packageLink({ ...pkg("a", "MIT"), homepage: "https://h" })).toBe("https://h");
    expect(packageLink(pkg("a", "MIT"))).toBeNull();
  });
});

describe("isSupported", () => {
  it("accepts schema v1 only", () => {
    expect(isSupported({ schemaVersion: 1, packages: [] })).toBe(true);
    expect(isSupported({ schemaVersion: 2, packages: [] })).toBe(false);
    expect(isSupported(null)).toBe(false);
    expect(isSupported({ schemaVersion: 1 })).toBe(false);
  });
});

describe("loadLicenses", () => {
  it("resolves to the generated notices, or null when they were not generated", async () => {
    const data = await loadLicenses();
    // Both are valid: `src/generated/licenses.json` is gitignored and made by `pnpm gen:licenses`.
    if (data) {
      expect(data.schemaVersion).toBe(1);
      expect(data.ffmpeg.name).toBe("FFmpeg");
      expect(data.packages.length).toBeGreaterThan(0);
    } else {
      expect(data).toBeNull();
    }
  });
});
