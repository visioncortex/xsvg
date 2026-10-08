#!/usr/bin/env node
// Bump the release version everywhere it lives, in one shot.
//
//   node scripts/bump.mjs 0.1.1
//
// The version lives in the 2 npm packages and every workspace crate, including the
// `version = "…"` on intra-workspace path deps so `cargo publish` stays happy. This
// rewrites those manifests, re-syncs the READMEs, refreshes Cargo.lock, and stages
// everything — then stops so you can review the diff before committing. Publishing
// stays a separate step (scripts/publish.sh). (The README badges are shields.io and
// read the registries, so there is nothing to regenerate.)

import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const R = (p) => resolve(root, p);

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`usage: node scripts/bump.mjs <version>   (semver, e.g. 0.1.1)`);
  console.error(version ? `  '${version}' is not a valid version` : "  no version given");
  process.exit(1);
}

const NPM = ["packages/xsvg-viewer/package.json", "packages/xsvg-compile/package.json"];
// Every workspace member, read from cargo itself, so a new crate cannot be missed.
const CRATES = JSON.parse(
  execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    cwd: root,
    encoding: "utf8",
  }),
).packages.map((p) => p.manifest_path.slice(root.length + 1));

const oldVersion = JSON.parse(readFileSync(R(NPM[0]), "utf8")).version;
if (oldVersion === version) {
  console.error(`already at ${version} — nothing to do`);
  process.exit(1);
}

const edit = (rel, fn) => {
  const before = readFileSync(R(rel), "utf8");
  const after = fn(before);
  if (after === before) throw new Error(`no version field changed in ${rel}`);
  writeFileSync(R(rel), after);
  console.log(`  ${rel}`);
};

console.log(`bump ${oldVersion} → ${version}`);

// npm: the only top-level "version" key (dependency values aren't keyed "version").
for (const p of NPM) edit(p, (s) => s.replace(/("version":\s*")[^"]*(")/, `$1${version}$2`));

// crates: the package version (only `version = "…"` at column 0), plus every
// intra-workspace dep — an inline table carrying both a `path = "../…"` and a
// `version`, in either order — so external deps (which have no path) are never
// touched, whatever the number is.
for (const c of CRATES)
  edit(c, (s) =>
    s
      .replace(/^version = "[^"]*"/m, `version = "${version}"`)
      .replace(/(path = "\.\.[^"]*",\s*version = ")[^"]*(")/g, `$1${version}$2`)
      .replace(/(version = ")[^"]*(",\s*path = "\.\.)/g, `$1${version}$2`)
  );

const run = (cmd, args) => {
  console.log(`  $ ${cmd} ${args.join(" ")}`);
  execFileSync(cmd, args, { cwd: root, stdio: ["ignore", "ignore", "inherit"] });
};

console.log("syncing READMEs:");
run("npm", ["run", "sync:readme"]);

console.log("refreshing package-lock.json:");
run("npm", ["install", "--package-lock-only", "--ignore-scripts", "--offline"]);

console.log("refreshing Cargo.lock:");
try {
  run("cargo", ["update", "--workspace", "--offline"]);
} catch {
  try {
    run("cargo", ["metadata", "--format-version", "1", "--quiet"]);
  } catch {
    console.warn("  ! could not refresh Cargo.lock offline — run `cargo update --workspace` before committing");
  }
}

console.log("staging changes:");
try {
  run("git", ["add", "-A", "--", ...NPM, ...CRATES, "Cargo.lock", "package-lock.json", "packages", "crates"]);
  console.log(`\n✓ bumped to ${version} and staged. Review with \`git diff --staged\`, then commit and run scripts/publish.sh`);
} catch {
  console.warn("  ! git add failed — stage the changes manually");
}
