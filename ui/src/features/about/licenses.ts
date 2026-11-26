/**
 * Third-party licence notices for the About dialog (F0.10). `scripts/gen-licenses.mjs` writes
 * `src/generated/licenses.json` at build time (`pnpm gen:licenses`, part of `pnpm dev`/`build`).
 * It is not a Rust IPC type, so the schema is written here by hand; it must match the script.
 */

export type LicenseEcosystem = "rust" | "cargo" | "npm";

export type ThirdPartyPackage = {
  ecosystem: LicenseEcosystem;
  name: string;
  version: string;
  /** SPDX expression, e.g. `MIT OR Apache-2.0`. */
  license: string;
  repository: string | null;
  homepage: string | null;
  authors: string[];
  /** Keys of `licenseTexts`. */
  licenseTextIds: string[];
};

export type FfmpegNotice = {
  name: "FFmpeg";
  version: string;
  branch: string;
  variant: string;
  buildTag: string;
  license: string;
  sourceCode: string;
  buildScripts: string;
  binaryUrl: string;
  sha256: string;
  dlls: string[];
  notice: string;
  sourceOffer: string;
  licenseTextIds: string[];
};

export type LicenseText = { name: string; text: string };

export type ThirdPartyLicenses = {
  schemaVersion: 1;
  ffmpeg: FfmpegNotice;
  packages: ThirdPartyPackage[];
  licenseTexts: Record<string, LicenseText>;
};

export type LicensesLoader = () => Promise<ThirdPartyLicenses | null>;

// A glob instead of a plain import: the file is gitignored and may be missing on a clean checkout,
// and a glob that matches nothing still typechecks and builds. The JSON gets its own chunk, loaded
// only when the dialog shows the licences.
const loaders = import.meta.glob<ThirdPartyLicenses>("../../generated/licenses.json", { import: "default" });
const generated = loaders["../../generated/licenses.json"];

/** `null` when the notices were not generated, or by a script with another schema. */
export const loadLicenses: LicensesLoader = async () => {
  if (!generated) return null;
  const data = await generated();
  return isSupported(data) ? data : null;
};

/** The dialog reads exactly schema v1; anything else is shown as "not generated". */
export function isSupported(data: unknown): data is ThirdPartyLicenses {
  return (
    typeof data === "object" &&
    data !== null &&
    "schemaVersion" in data &&
    data.schemaVersion === 1 &&
    "packages" in data &&
    Array.isArray(data.packages)
  );
}

/** Stable key of a package row (the same crate can appear in two versions). */
export function packageKey(pkg: ThirdPartyPackage): string {
  return `${pkg.ecosystem}:${pkg.name}@${pkg.version}`;
}

/**
 * The packages matching `query`: every whitespace-separated word must appear (case-insensitive) in
 * the name, version, licence or ecosystem. An empty query matches everything. Order is kept.
 */
export function filterPackages(packages: readonly ThirdPartyPackage[], query: string): ThirdPartyPackage[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...packages];
  return packages.filter((pkg) => {
    const haystack = `${pkg.name} ${pkg.version} ${pkg.license} ${pkg.ecosystem}`.toLowerCase();
    return words.every((word) => haystack.includes(word));
  });
}

/** Rows rendered at once; "Show more" adds another page (the list has ~330 packages). */
export const LICENSE_PAGE_SIZE = 50;

/** The first `limit` rows and how many are left for "Show more". */
export function pageOf<T>(rows: readonly T[], limit: number): { visible: T[]; remaining: number } {
  const visible = rows.slice(0, Math.max(0, limit));
  return { visible, remaining: rows.length - visible.length };
}

/** The licence texts of a package or FFmpeg, skipping ids the file does not contain. */
export function textsOf(data: ThirdPartyLicenses, ids: readonly string[]): (LicenseText & { id: string })[] {
  return ids.flatMap((id) => {
    const text = Object.hasOwn(data.licenseTexts, id) ? data.licenseTexts[id] : undefined;
    return text ? [{ id, ...text }] : [];
  });
}

/** The link shown for a package: its repository, else its homepage. */
export function packageLink(pkg: ThirdPartyPackage): string | null {
  return pkg.repository ?? pkg.homepage;
}
