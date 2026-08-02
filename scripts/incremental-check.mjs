// Checks the incremental editing protocol end to end (docs/Incremental.md §4).
//
// The engine's own invariant — that a fragment compile is a verbatim slice of the
// full compile — is pinned in Rust. What is *not* pinned there is the half the
// viewer implements in JS: naming every element, carrying those names across an
// edit, and locating the run of rendered nodes each re-emitted fragment replaces.
//
// So this reconstructs a patch the way the viewer does, purely as strings, and
// demands the result be byte-identical to a full recompile of the edited source.
// A discrepancy here is a viewer that silently renders something the document does
// not say. It also checks the property host keys exist for — that an element nobody
// touched compiles to identical bytes after an edit moved it — directly, rather than
// inferring it from a patch that happened to succeed. Run with:
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
  nodeSpans,
  normalizeSource,
} from "../packages/xsvg-compile/dist/index.js";
import { singleEdit } from "../packages/xsvg-viewer/src/sourcemap.ts";
import { NodeKeyStore } from "../packages/xsvg-viewer/src/keys.ts";

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

// ---- identity spellings stay funneled -------------------------------------

check("identity attributes are read only through the sanctioned funnels", () => {
  // There are two identity spellings — `data-xsvg-pos` byte ranges and
  // `data-xsvg-node` host keys — and every consumer must handle both. That stays
  // manageable only while the handling is funneled: `posAttr` (sourcemap.ts)
  // resolves either spelling, the inspector tracks its pin through one `identity`
  // helper, and the patch path anchors on names in patch.ts. A new call site
  // reading the attributes directly would handle one spelling and silently break
  // on documents using the other, so it fails here instead.
  const sanctioned = new Set(["sourcemap.ts", "inspector.ts", "patch.ts"]);
  const reads =
    /(?:getAttribute|setAttribute|hasAttribute|closest|querySelector(?:All)?)\s*\(\s*[`"'][^`"')]*data-xsvg-(?:pos|node)/;
  for (const pkg of ["xsvg-viewer", "xsvg-compile"]) {
    const dir = resolve(import.meta.dirname, `../packages/${pkg}/src`);
    for (const file of readdirSync(dir).filter((f) => f.endsWith(".ts"))) {
      if (sanctioned.has(file)) continue;
      const hit = readFileSync(join(dir, file), "utf8").match(reads);
      assert.equal(
        hit,
        null,
        `${pkg}/src/${file} reads an identity attribute directly (${hit?.[0]}); ` +
          `resolve it through posAttr/nearestSource or the patch path instead`,
      );
    }
  }
});

// ---- the protocol, against real documents ---------------------------------

/**
 * Rebuild a rendered document for an edited source the way the viewer patches its
 * DOM, and hand back the result for comparison against a real recompile.
 *
 * Models the **keyed** path, because that is what ships: every element is named
 * before compiling, the names ride across the edit, and the anchor is a name rather
 * than a byte range. What is conspicuously absent is the bookkeeping the unkeyed
 * path needed — no span re-stamping, and no declining because a generated def id
 * would go stale. Untouched output is untouched, byte for byte.
 *
 * Returns `{ skipped }` for the cases the viewer itself declines — those fall back
 * to a full compile, where there is nothing to verify.
 */
function patch(source, next, o) {
  const edit = singleEdit(source, next);
  if (!edit) return { skipped: "no change" };

  // Key assignment is a deterministic walk in document order, so a store synced to
  // the same text always produces the same names — the harness can rebuild the
  // "rendered" document rather than thread one through.
  const keys = new NodeKeyStore();
  keys.sync(nodeSpans(source));
  const before = { ...o, keys: keys.key };
  const rendered = compile(source, before);
  // The store names every element, and the compiler seconds that by staying silent:
  // a keyed compile that had to fall back to byte spans says so in its own output.
  assert.ok(!rendered.includes("element(s) unkeyed"), "the key store missed an element");

  // The common prefix ends at one offset in both versions; it is the pivot.
  const pivot = bytes(next.slice(0, edit.from));
  const delta = bytes(next) - bytes(source);

  const unit = fragmentRange(next, pivot);
  if (!unit) return { skipped: "edit is not inside a top-level element" };
  const targets = [unit, ...dependents(next, pivot)];

  // Reproduce each target as it currently stands, while the store still describes
  // the document as rendered.
  const current = targets.map(([start]) =>
    compileFragment(source, start <= pivot ? start : start - delta, before),
  );

  keys.shift(pivot, delta);
  keys.sync(nodeSpans(next));
  const after = { ...o, keys: keys.key };

  const swaps = [];
  for (const [i, [start]] of targets.entries()) {
    const was = current[i];
    if (was == null) return { skipped: "fragment had no previous emission" };
    const name = keys.key(start);
    if (!name) return { skipped: "fragment has no name" };
    const anchor = `data-xsvg-node="${name}"`;
    // A fragment carrying no anchor of its own — an element that compiles to nothing,
    // like a declaration or a hidden layer — is unaddressable, and the viewer
    // recompiles rather than guessing.
    if (!was.includes(anchor)) return { skipped: "fragment carries no anchor of its own" };
    assert.equal(was.split(anchor).length - 1, 1, `fragment ${name} claims its own name twice`);
    // Locating the run by its markup is the string equivalent of walking siblings
    // from the anchor: both rest on the fragment being a verbatim slice of the whole.
    const at = rendered.indexOf(was);
    assert.ok(at >= 0, `fragment ${name} is not a slice of the rendered output`);
    assert.equal(rendered.indexOf(was, at + 1), -1, `fragment ${name} is not unique in the output`);
    const now = compileFragment(next, start, after);
    if (now == null) return { skipped: "fragment cannot be re-emitted" };
    swaps.push({ at, len: was.length, now });
  }

  swaps.sort((x, y) => x.at - y.at);
  let out = "";
  let cursor = 0;
  for (const s of swaps) {
    assert.ok(s.at >= cursor, "fragments to replace overlap");
    out += rendered.slice(cursor, s.at) + s.now;
    cursor = s.at + s.len;
  }
  return {
    out: out + rendered.slice(cursor),
    expected: compile(next, after),
    shifted: delta !== 0,
  };
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

/**
 * The property host keys exist for: an element nobody touched compiles to the same
 * bytes after an edit that moved it (docs/Incremental.md §3.1).
 *
 * Checked directly rather than inferred from a successful patch, because a patch can
 * succeed while quietly depending on the host having chased a rename. If this holds,
 * the whole class of chasing — span re-stamping, def-id tracking — is unnecessary
 * rather than merely working.
 */
function keyedStabilityHolds(source, next, o) {
  const keys = new NodeKeyStore();
  keys.sync(nodeSpans(source));
  const before = new Map();
  for (const [start] of nodeSpans(source)) {
    const r = fragmentRange(source, start);
    if (r && r[0] === start) before.set(keys.key(start), compileFragment(source, start, { ...o, keys: keys.key }));
  }

  const edit = singleEdit(source, next);
  const pivot = bytes(next.slice(0, edit.from));
  const delta = bytes(next) - bytes(source);
  keys.shift(pivot, delta);
  keys.sync(nodeSpans(next));

  // Everything the edit can legitimately have changed.
  const unit = fragmentRange(next, pivot);
  if (!unit) return { skipped: true };
  const dirty = new Set(
    [unit, ...dependents(next, pivot)].map(([s]) => keys.key(s)).filter(Boolean),
  );

  let compared = 0;
  for (const [start] of nodeSpans(next)) {
    const r = fragmentRange(next, start);
    if (!r || r[0] !== start) continue;
    const name = keys.key(start);
    if (!name || dirty.has(name) || !before.has(name)) continue;
    const after = compileFragment(next, start, { ...o, keys: keys.key });
    assert.equal(
      after,
      before.get(name),
      `element ${name} was not edited, yet its output changed — keys did not make it offset-independent`,
    );
    compared++;
  }
  return { compared };
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
// Unedited elements confirmed to compile to identical bytes after an edit moved them —
// the property host keys exist for.
let stable = 0;

for (const file of samples) {
  const path = join(DATASET, file);
  const o = { ...opts, basePath: path };
  // Normalize up front: every offset below indexes the text the compiler sees, not
  // the text on disk (see `normalizeSource`).
  const source = normalizeSource(readFileSync(path, "utf8"));

  const candidates = edits(source);
  // Spread the picks across the document instead of taking the first few, which
  // would only ever exercise the root element's neighbourhood.
  const stride = Math.max(1, Math.floor(candidates.length / PER_SAMPLE));
  for (let i = 0; i < candidates.length; i += stride) {
    const c = candidates[i];
    check(`${file}: ${c.label}`, () => {
      const result = patch(source, c.source, o);
      if (result.skipped) {
        skipped++;
        return;
      }
      patched++;
      if (result.shifted) shifting++;
      assert.equal(
        result.out,
        result.expected,
        "patched output differs from a full recompile of the edited source",
      );
      // Only meaningful when something moved; a same-length edit is stable trivially.
      if (result.shifted) {
        const k = keyedStabilityHolds(source, c.source, o);
        if (k.compared) stable += k.compared;
      }
    });
  }
}

console.log(
  `incremental: ${passed} checks passed (${patched} patches verified byte-identical, ` +
    `${shifting} of them shifting every later offset; ${skipped} declined to a full ` +
    `compile) over ${samples.length} samples\n` +
    `             ${stable} unedited elements confirmed byte-stable across a shifting edit`,
);
if (failed) console.error(`${failed} failed`);
