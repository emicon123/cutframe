#!/usr/bin/env node
// Generates the third-party licence notices shown in the About dialog (task 0140, F0.10).
//
//   pnpm gen:licenses        -> ui/src/generated/licenses.json (gitignored, rebuilt by pnpm dev/build/check)
//
// Sources (all local, no network beyond what `cargo metadata` needs to fetch crate sources once):
//   - Rust: `cargo metadata --filter-platform x86_64-pc-windows-msvc`, walked from the `cutframe`
//     binary crate (src-tauri) over normal dependencies only. Dev-dependencies, build-dependencies
//     and proc-macro crates (with whatever only they pull in) are left out: they run at compile
//     time and are not linked into Cutframe.exe. Our own workspace crates are left out too. The
//     Rust standard library, which is linked in, gets one entry of its own.
//   - npm: `pnpm licenses list --json --prod` in ui/ (dependencies, not devDependencies: what Vite
//     bundles into the production build).
//   - FFmpeg: scripts/ffmpeg.json, plus .ffmpeg/LICENSE.txt and .ffmpeg/NOTICE.txt when provisioned
//     (pnpm setup:ffmpeg); without them the notice is built from ffmpeg.json alone.
//   Licence texts come from each package's own LICENSE/COPYING/NOTICE files. A package that ships
//   none falls back to the canonical SPDX text in scripts/licenses/spdx/ (license-list-data v3.27.0).
//
// The script fails (exit 1) when a package has no licence expression, no licence text can be
// found, or a bundled npm package is GPL/AGPL/LGPL-only. Fix by adding an entry to OVERRIDES below
// (after checking the package's real licence), never by weakening the check.
//
// Output schema (schemaVersion 1), consumed by the About dialog:
//   {
//     schemaVersion: 1,
//     ffmpeg: { name, version, branch, variant, buildTag, license, sourceCode, buildScripts,
//               binaryUrl, sha256, dlls: string[], notice, sourceOffer, licenseTextIds: string[] },
//     packages: [{ ecosystem: "cargo" | "npm" | "rust", name, version, license (SPDX expression),
//                  repository: string | null, homepage: string | null, authors: string[],
//                  licenseTextIds: string[] }],      // sorted by ecosystem, name, version
//     licenseTexts: { [id]: { name, text } }        // deduplicated; id = sha-256 prefix
//   }

import { createHash } from "node:crypto";
import { execSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SPDX_DIR = join(ROOT, "scripts", "licenses", "spdx");
const OUT = join(ROOT, "ui", "src", "generated", "licenses.json");
const TARGET = "x86_64-pc-windows-msvc";

// Manual licence data for packages whose metadata is missing or wrong. Key: "<ecosystem>:<name>".
// Each entry must cite where the licence was verified.
const OVERRIDES = {};

const COPYLEFT = /^(A|L)?GPL-/;
const LICENSE_FILE = /^(licen[cs]e|copying|copyright|notice|unlicense)([-._].*)?$/i;

function run(cmd, cwd) {
  return execSync(cmd, { cwd, encoding: "utf8", maxBuffer: 512 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
}

// --- licence texts --------------------------------------------------------------------------------

const texts = new Map(); // id -> { name, text }

function addText(name, raw) {
  const text = raw.replace(/^﻿/, "").replace(/\r\n?/g, "\n").trim();
  if (!text) return null;
  const key = text.replace(/\s+/g, " ").toLowerCase();
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  if (!texts.has(id)) texts.set(id, { name, text });
  return id;
}

const spdxCache = new Map();
function spdxText(id) {
  if (!spdxCache.has(id)) {
    const file = join(SPDX_DIR, `${id}.txt`);
    spdxCache.set(id, existsSync(file) ? addText(id, readFileSync(file, "utf8")) : null);
  }
  return spdxCache.get(id);
}

function licenseFilesIn(dir) {
  if (!dir || !existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => addText(f, readFileSync(join(dir, f), "utf8")))
    .filter(Boolean);
}

/** SPDX ids in an expression, e.g. "(MIT OR Apache-2.0) AND Unicode-3.0" -> [MIT, Apache-2.0, Unicode-3.0]. */
function spdxIds(expr) {
  return expr
    .replace(/[()]/g, " ")
    .split(/\s+(?:OR|AND|WITH)\s+|\s+/)
    .map((s) => s.trim().replace(/\+$/, ""))
    .filter(Boolean);
}

/** Old crates write "MIT/Apache-2.0"; SPDX says "MIT OR Apache-2.0". */
function normalizeExpr(expr) {
  return expr.trim().replace(/\s*\/\s*/g, " OR ");
}

const problems = [];

function finish(pkg, dir, extraFiles = []) {
  const override = OVERRIDES[`${pkg.ecosystem}:${pkg.name}`];
  if (override) Object.assign(pkg, override);
  if (!pkg.license || /^(unknown|unlicensed|see licen[cs]e)/i.test(pkg.license)) {
    problems.push(`${pkg.ecosystem}:${pkg.name}@${pkg.version}: no SPDX licence expression ("${pkg.license}")`);
  }
  const ids = new Set([...extraFiles, ...licenseFilesIn(dir)]);
  if (ids.size === 0 && pkg.license) {
    for (const spdx of spdxIds(pkg.license)) {
      const id = spdxText(spdx);
      if (id) ids.add(id);
    }
  }
  if (ids.size === 0) problems.push(`${pkg.ecosystem}:${pkg.name}@${pkg.version}: no licence text found (${pkg.license})`);
  pkg.licenseTextIds = [...ids];
  return pkg;
}

// --- Rust crates ----------------------------------------------------------------------------------

function cargoPackages() {
  const meta = JSON.parse(run(`cargo metadata --format-version 1 --locked --filter-platform ${TARGET}`, ROOT));
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const rootId = meta.workspace_members.find((id) => byId.get(id).manifest_path.replace(/\\/g, "/").endsWith("src-tauri/Cargo.toml"));
  if (!rootId) throw new Error("cargo metadata: src-tauri (cutframe) not found among the workspace members");

  const seen = new Set([rootId]);
  const queue = [rootId];
  const out = [];
  while (queue.length) {
    const id = queue.shift();
    for (const dep of nodes.get(id)?.deps ?? []) {
      if (seen.has(dep.pkg)) continue;
      if (!dep.dep_kinds.some((k) => k.kind === null)) continue; // dev/build only
      const p = byId.get(dep.pkg);
      if (p.targets.some((t) => t.kind.includes("proc-macro"))) continue; // compile time only
      seen.add(dep.pkg);
      queue.push(dep.pkg);
      if (members.has(dep.pkg) || p.source === null) continue; // our own crates
      const dir = dirname(p.manifest_path);
      const extra = p.license_file ? [addText(p.license_file, readFileSync(join(dir, p.license_file), "utf8"))] : [];
      out.push(
        finish(
          {
            ecosystem: "cargo",
            name: p.name,
            version: p.version,
            license: p.license ? normalizeExpr(p.license) : null,
            repository: p.repository ?? null,
            homepage: p.homepage ?? null,
            authors: p.authors ?? [],
          },
          dir,
          extra.filter(Boolean),
        ),
      );
    }
  }
  return out;
}

function rustStd() {
  const version = run("rustc --version", ROOT).trim().split(/\s+/)[1] ?? "unknown";
  return finish(
    {
      ecosystem: "rust",
      name: "Rust standard library",
      version,
      license: "MIT OR Apache-2.0",
      repository: "https://github.com/rust-lang/rust",
      homepage: "https://www.rust-lang.org",
      authors: ["The Rust Project Developers"],
    },
    null,
  );
}

// --- npm packages ---------------------------------------------------------------------------------

function repoUrl(repo) {
  const url = typeof repo === "string" ? repo : repo?.url;
  if (!url) return null;
  if (/^[\w.-]+\/[\w.-]+$/.test(url)) return `https://github.com/${url}`; // "owner/name" shorthand
  return url.replace(/^git\+/, "").replace(/^git:\/\//, "https://").replace(/^ssh:\/\/git@/, "https://").replace(/\.git$/, "");
}

/** True when some OR alternative of the expression contains no copyleft licence. */
function hasPermissiveChoice(expr) {
  return expr
    .replace(/[()]/g, " ")
    .split(/\s+OR\s+/)
    .some((alt) => !spdxIds(alt).some((id) => COPYLEFT.test(id)));
}

function npmPackages() {
  const list = JSON.parse(run("pnpm licenses list --json --prod", join(ROOT, "ui")));
  const out = [];
  for (const entries of Object.values(list)) {
    for (const e of entries) {
      e.versions.forEach((version, i) => {
        const dir = e.paths[i];
        const pj = dir && existsSync(join(dir, "package.json")) ? JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) : {};
        const author = typeof pj.author === "string" ? pj.author : pj.author?.name;
        const pkg = finish(
          {
            ecosystem: "npm",
            name: e.name,
            version,
            license: typeof e.license === "string" && e.license !== "Unknown" ? normalizeExpr(e.license) : null,
            repository: repoUrl(pj.repository),
            homepage: e.homepage ?? pj.homepage ?? null,
            authors: author ? [author] : [],
          },
          dir,
        );
        if (pkg.license && !hasPermissiveChoice(pkg.license)) {
          problems.push(`npm:${pkg.name}@${version}: copyleft licence ${pkg.license} in the UI bundle`);
        }
        out.push(pkg);
      });
    }
  }
  return out;
}

// --- FFmpeg ---------------------------------------------------------------------------------------

function ffmpeg() {
  const pin = JSON.parse(readFileSync(join(ROOT, "scripts", "ffmpeg.json"), "utf8"));
  const dir = join(ROOT, ".ffmpeg");
  const noticeFile = join(dir, "NOTICE.txt");
  const licenseFile = join(dir, "LICENSE.txt");
  // Same wording as scripts/fetch-ffmpeg.ps1 writes into NOTICE.txt, minus the configure line,
  // which only the provisioned DLLs can tell.
  const notice = existsSync(noticeFile)
    ? readFileSync(noticeFile, "utf8").replace(/\r\n?/g, "\n").trim()
    : `Cutframe uses the FFmpeg libraries (libavcodec, libavdevice, libavfilter, libavformat, libavutil,
libswresample, libswscale), version ${pin.version} (release branch ${pin.branch}), licensed under the
GNU Lesser General Public License version 3 or later (LGPL-3.0-or-later). FFmpeg is a trademark of
Fabrice Bellard, originator of the FFmpeg project. Cutframe is not affiliated with the FFmpeg project.

The libraries are unmodified builds, dynamically linked, and installed as separate DLL files next to
Cutframe.exe (${pin.dlls.join(", ")}).
You may replace them with your own build of the same FFmpeg release branch.

Corresponding source code
  FFmpeg source:        ${pin.sourceCode}
  Build scripts:        ${pin.buildScripts}
  Binary archive:       ${pin.url}
  Archive SHA-256:      ${pin.sha256}`;
  const licenseTextIds = [spdxText("LGPL-3.0-or-later")];
  if (existsSync(licenseFile)) licenseTextIds.push(addText("FFmpeg LICENSE.txt", readFileSync(licenseFile, "utf8")));
  return {
    name: "FFmpeg",
    version: pin.version,
    branch: pin.branch,
    variant: pin.variant,
    buildTag: pin.buildTag,
    license: pin.license,
    sourceCode: pin.sourceCode,
    buildScripts: pin.buildScripts,
    binaryUrl: pin.url,
    sha256: pin.sha256,
    dlls: pin.dlls,
    notice,
    sourceOffer:
      `The complete corresponding source code of the FFmpeg libraries shipped with this copy of Cutframe ` +
      `(FFmpeg ${pin.version}, built by the scripts at ${pin.buildScripts}) is available at ${pin.sourceCode}. ` +
      `For at least three years after this release, the distributor of this copy of Cutframe will also provide ` +
      `that source on request, for no more than the cost of physically performing the transfer.`,
    licenseTextIds: [...new Set(licenseTextIds.filter(Boolean))],
  };
}

// --- main -----------------------------------------------------------------------------------------

const ffmpegEntry = ffmpeg();
const order = { rust: 0, cargo: 1, npm: 2 };
const packages = [rustStd(), ...cargoPackages(), ...npmPackages()].sort(
  (a, b) => order[a.ecosystem] - order[b.ecosystem] || a.name.localeCompare(b.name, "en") || a.version.localeCompare(b.version, "en"),
);

if (problems.length) {
  console.error(`gen-licenses: ${problems.length} problem(s):\n  ${problems.join("\n  ")}`);
  process.exit(1);
}

const used = new Set([...ffmpegEntry.licenseTextIds, ...packages.flatMap((p) => p.licenseTextIds)]);
const licenseTexts = Object.fromEntries([...texts].filter(([id]) => used.has(id)).sort(([a], [b]) => a.localeCompare(b)));

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, `${JSON.stringify({ schemaVersion: 1, ffmpeg: ffmpegEntry, packages, licenseTexts })}\n`);

const count = (eco) => packages.filter((p) => p.ecosystem === eco).length;
console.log(
  `gen-licenses: ${count("cargo")} crates, ${count("npm")} npm packages, Rust std, FFmpeg ${ffmpegEntry.version}; ` +
    `${Object.keys(licenseTexts).length} distinct licence texts -> ${relative(ROOT, OUT)} ` +
    `(${Math.round(statSync(OUT).size / 1024)} KiB)`,
);
