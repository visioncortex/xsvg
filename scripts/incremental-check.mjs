// Checks the incremental editing protocol end to end (docs/Incremental.md §4).
//
// The engine's own invariant — that a fragment compile is a verbatim slice of the
// full compile — is pinned in Rust. What is *not* pinned there is the half the
// viewer implements in JS: locating the fragment an edit falls in, mapping source
// offsets across the edit so the already-rendered output can be re-anchored, and
// re-stamping the spans of everything that did not change.
//
// So this reconstructs a patch the way the viewer does, purely as strings, and
// demands the result be byte-identical to a full recompile of the edited source.
// A discrepancy here is a viewer that silently renders something the document does
// not say. Run with:
//
//   npm run check:incremental
//
// (which builds @visioncortex/xsvg-compile first — this drives the published Node
// entry points, not a stand-in, so the protocol is checked as shipped.)

import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  compile,
  compileFragment,
  dependents,
  fragmentRange,
  normalizeSource,
} from "../packages/xsvg-compile/dist/index.js";
import { singleEdit } from "../packages/xsvg-viewer/src/sourcemap.ts";

const DATASET = resolve(import.meta.dirname, "../dataset");
const FONTS = resolve(import.meta.dirname, "../assets/fonts");
const opts = { sourcemap: true, fontDir: existsSync(FONTS) ? FONTS : undefined };

let passed = 0;
let failed = 0;
function check(name, fn) {
  try {
    fn();
    passed++;
  } catch (e) {
    console.error(`FAIL  ${name}\n      ${e.message.split("\n")[0]}`);
    failed++;
    process.exitCode = 1;
  }
}

const bytes = (s) => Buffer.byteLength(s, "utf8");

// ---- the offset arithmetic, on its own ------------------------------------

check("singleEdit isolates the changed span", () => {
  assert.equal(singleEdit("abc", "abc"), null);

  // Stated as what the spans *cover*, so the expectation is read off the strings
  // rather than counted out by hand.
  const spans = (a, b) => {
    const e = singleEdit(a, b);
    // The whole point: splicing the new span over the old one reproduces `b`.
    assert.equal(a.slice(0, e.from) + b.slice(e.from, e.insertEnd) + a.slice(e.to), b);
    return [a.slice(e.from, e.to), b.slice(e.from, e.insertEnd)];
  };

  assert.deepEqual(spans(`w="10"`, `w="12"`), ["0", "2"]);
  // A pure insertion inside a run of equal characters must not drift: the suffix
  // scan is capped so the two spans can never cross.
  assert.deepEqual(spans(`w="10"`, `w="100"`), ["", "0"]);
  assert.deepEqual(spans(`w="100"`, `w="10"`), ["0", ""]);
  // Deletion to empty, and growth at the very end.
  assert.deepEqual(spans(`ab`, ``), ["ab", ""]);
  assert.deepEqual(spans(`ab`, `abc`), ["", "c"]);
});

check("singleEdit spans a scattered change rather than mis-describing it", () => {
  // Two separate changes collapse into one covering span — wider than necessary,
  // never wrong. The viewer then finds it crosses fragments and recompiles.
  const old = `<a x="1"/><b y="1"/>`;
  const e = singleEdit(old, `<a x="2"/><b y="3"/>`);
  assert.equal(old.slice(0, e.from), `<a x="`);
  assert.equal(old.slice(e.from, e.to), `1"/><b y="1`);
});

// ---- the protocol, against real documents ---------------------------------

/** Rewrite the source spans a patch leaves in place: everything after the edit
 *  moved by the edit's length, everything before it did not. */
const restamp = (text, toNew) =>
  text.replace(
    /data-xsvg-pos="(\d+)-(\d+)"/g,
    (_m, a, b) => `data-xsvg-pos="${toNew(+a)}-${toNew(+b)}"`,
  );

/**
 * Rebuild a rendered document for an edited source the way the viewer patches its
 * DOM, and hand back the result for comparison against a real recompile.
 *
 * Returns `{ skipped }` for the cases the viewer itself declines — those fall back
 * to a full compile, where there is nothing to verify.
 */
function patch(rendered, source, next, o) {
  const edit = singleEdit(source, next);
  if (!edit) return { skipped: "no change" };

  // The common prefix ends at one offset in both versions; it is the pivot every
  // other offset is mapped around.
  const pivot = bytes(next.slice(0, edit.from));
  const delta = bytes(next) - bytes(source);
  const toOld = (b) => (b <= pivot ? b : b - delta);
  const toNew = (b) => (b <= pivot ? b : b + delta);

  const unit = fragmentRange(next, pivot);
  if (!unit) return { skipped: "edit is not inside a top-level element" };
  const targets = [unit, ...dependents(next, pivot)];

  const swaps = [];
  for (const [start, end] of targets) {
    const before = compileFragment(source, toOld(start), o);
    if (before == null) return { skipped: "fragment had no previous emission" };
    const key = `data-xsvg-pos="${toOld(start)}-${toOld(end)}"`;
    // The viewer anchors on the fragment's own range, then takes the whole run of
    // sibling nodes around it. A fragment carrying no such key — an element that
    // compiles to nothing, like a declaration or a hidden layer — is unaddressable,
    // and the viewer recompiles rather than guessing.
    if (!before.includes(key)) return { skipped: "fragment carries no anchor of its own" };
    assert.equal(
      before.split(key).length - 1,
      1,
      `fragment ${toOld(start)}-${toOld(end)} claims its own range more than once`,
    );
    // Locating the run by its markup is the string equivalent of walking siblings
    // from the anchor: both rest on the fragment being a verbatim slice of the whole.
    const at = rendered.indexOf(before);
    assert.ok(at >= 0, `fragment ${toOld(start)}-${toOld(end)} is not a slice of the rendered output`);
    assert.equal(
      rendered.indexOf(before, at + 1),
      -1,
      `fragment markup at ${toOld(start)}-${toOld(end)} is not unique in the output`,
    );
    const after = compileFragment(next, start, o);
    if (after == null) return { skipped: "fragment cannot be re-emitted" };
    swaps.push({ at, len: before.length, after });
  }

  swaps.sort((x, y) => x.at - y.at);

  // Generated def ids (`x-flt-…`, `x-mesh-…`, `x-mgc-…`) seed on the node's byte
  // start, so an offset shift renames them — and a fragment we do not re-emit keeps
  // the old name. The viewer declines rather than reimplement the compiler's id
  // scheme; model that here, or the harness reports a divergence the shipped code
  // never produces. `NodeKeys` removes the offset-dependence for good.
  if (delta !== 0) {
    let outside = "";
    let end = 0;
    for (const s of swaps) {
      outside += rendered.slice(end, s.at);
      end = s.at + s.len;
    }
    outside += rendered.slice(end);
    if (/\bid="x-/.test(outside)) return { skipped: "would strand a byte-seeded def id" };
  }

  let out = "";
  let cursor = 0;
  for (const s of swaps) {
    assert.ok(s.at >= cursor, "fragments to replace overlap");
    // Re-emitted markup already carries new-source spans; everything between the
    // replacements has to be shifted onto them.
    out += restamp(rendered.slice(cursor, s.at), toNew) + s.after;
    cursor = s.at + s.len;
  }
  return { out: out + restamp(rendered.slice(cursor), toNew), shifted: delta !== 0 };
}

/**
 * Plausible single-attribute edits inside top-level elements — the shape every edit
 * the parameter panel makes actually takes.
 *
 * Deliberately produces a **length-changing** edit for every attribute, alongside the
 * same-length one. An edit that keeps the value's width shifts no offsets at all, so
 * it exercises none of the mapping this protocol is made of: the pivot arithmetic, the
 * re-anchoring, the re-stamping. Nine in ten arithmetic tweaks happen to keep their
 * width, so left to chance the interesting case is nearly absent.
 */
function edits(source) {
  const out = [];
  for (const m of source.matchAll(/([\w:-]+)="([^"<>]*)"/g)) {
    const valueAt = m.index + m[1].length + 2;
    // Only attributes on an element the fragment API can address on its own.
    const unit = fragmentRange(source, bytes(source.slice(0, m.index)));
    if (!unit) continue;
    const v = m[2];
    const nexts = [];
    if (/^-?\d+(\.\d+)?$/.test(v)) {
      nexts.push(`${v}0`); // one byte longer — every later offset moves
      const same = String(Number(v) + 7);
      if (same.length === v.length) nexts.push(same);
    } else if (v === "") {
      nexts.push("1");
    }
    for (const next of nexts) {
      out.push({
        label: `${m[1]}="${v}" → "${next}"${next.length === v.length ? "" : " [shifts]"}`,
        source: source.slice(0, valueAt) + next + source.slice(valueAt + v.length),
      });
    }
  }
  return out;
}

const samples = readdirSync(DATASET).filter((f) => f.endsWith(".xsvg")).sort();
const PER_SAMPLE = 6;
let patched = 0;
let skipped = 0;
// Patches where the edit changed the source's length, so every later offset moved.
// Reported because it is the number that matters: a same-length edit shifts nothing
// and exercises none of the mapping, and an earlier version of this harness was
// almost entirely made of those without saying so.
let shifting = 0;

for (const file of samples) {
  const path = join(DATASET, file);
  const o = { ...opts, basePath: path };
  // Normalize up front: every offset below indexes the text the compiler sees, not
  // the text on disk (see `normalizeSource`).
  const source = normalizeSource(readFileSync(path, "utf8"));
  const rendered = compile(source, o);

  const candidates = edits(source);
  // Spread the picks across the document instead of taking the first few, which
  // would only ever exercise the root element's neighbourhood.
  const stride = Math.max(1, Math.floor(candidates.length / PER_SAMPLE));
  for (let i = 0; i < candidates.length; i += stride) {
    const c = candidates[i];
    check(`${file}: ${c.label}`, () => {
      const result = patch(rendered, source, c.source, o);
      if (result.skipped) {
        skipped++;
        return;
      }
      patched++;
      if (result.shifted) shifting++;
      assert.equal(
        result.out,
        compile(c.source, o),
        "patched output differs from a full recompile of the edited source",
      );
    });
  }
}

console.log(
  `incremental: ${passed} checks passed (${patched} patches verified byte-identical, ` +
    `${shifting} of them shifting every later offset; ${skipped} declined to a full ` +
    `compile) over ${samples.length} samples`,
);
if (failed) console.error(`${failed} failed`);
