// Checks the DOM half of the incremental patch (packages/xsvg-viewer/src/patch.ts)
// against a real document tree.
//
// The string harness (incremental-check.mjs) deliberately reimplements the swap as
// string splicing, so it proves the *protocol* without touching the viewer's DOM
// walking. What it therefore never exercises is exactly what lives in patch.ts:
// locating a fragment's run of sibling nodes from its keyed anchor (a `<filter>`
// def ahead of its shape), splicing parsed nodes over it, and leaving every other
// node alone. A mistake there fails safe — the patch declines and the viewer
// recompiles — which means it would never *fail* anything, just quietly make every
// edit slow. So this drives the real `patchDocument` over a jsdom tree and demands
// the patched DOM serialize identically to a full recompile parsed fresh. Run with:
//
//   npm run check:dom

import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  compile,
  compileFragment,
  dependents,
  fragmentRange,
  nodeSpans,
} from "../packages/xsvg-compile/dist/index.js";
import { patchDocument } from "../packages/xsvg-viewer/src/patch.ts";
import { NodeKeyStore } from "../packages/xsvg-viewer/src/keys.ts";
import { ByteIndex, singleEdit } from "../packages/xsvg-viewer/src/sourcemap.ts";

// patch.ts reaches for `document` to parse fragment markup, exactly as it does in
// the browser; give it jsdom's.
const dom = new JSDOM("<!doctype html><body></body>");
globalThis.document = dom.window.document;

let passed = 0;
function check(name, fn) {
  try {
    fn();
    passed++;
  } catch (e) {
    console.error(`FAIL  ${name}\n      ${e.message.split("\n")[0]}`);
    process.exitCode = 1;
  }
}

// A filtered shape compiles to a run of sibling nodes (the `<filter>` def ahead of
// the `<rect>` that uses it) — the case the run walk exists for — plus a connector
// that bakes the shape's geometry, so the edit fans out to a dependent.
const SOURCE = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 400 300">
  <rect id="a" x="20" y="20" width="120" height="60" fill="#48a" filter="blur(3)"/>
  <rect id="b" x="260" y="180" width="100" height="50" fill="#f80"/>
  <x:connector from="#a:right" to="#b:left" stroke="#111"/>
</svg>`;

/** A rendered document in a fresh container, plus the pieces a patch needs. */
function render(source) {
  const keys = new NodeKeyStore();
  keys.sync(nodeSpans(source));
  const opts = { sourcemap: true, keys: keys.key };
  const container = document.createElement("div");
  container.innerHTML = compile(source, opts);
  // The session shape patchDocument wants, backed by the published Node entry
  // points — `opts.keys` reads the live store, exactly as the viewer's session does.
  const session = {
    source,
    range: (src, off) => fragmentRange(src, off),
    dependents: (src, off) => dependents(src, off),
    fragment: (src, off) => compileFragment(src, off, opts),
  };
  return { keys, opts, container, session, svg: container.querySelector("svg") };
}

check("a patched DOM serializes identically to a full recompile", () => {
  const next = SOURCE.replace(`x="20"`, `x="200"`); // length-changing: offsets shift
  const { keys, container, session, svg } = render(SOURCE);

  // Preconditions that make this test mean something: the edited fragment's run is
  // more than one node, and the edit reaches a dependent.
  assert.ok(container.innerHTML.includes("<filter"), "fixture lost its filter run");
  const edited = fragmentRange(SOURCE, SOURCE.indexOf(`id="a"`))[0];
  assert.equal(dependents(SOURCE, edited).length, 1, "fixture lost its dependent connector");

  const ok = patchDocument({ session, keys, spans: nodeSpans }, svg, {
    compiled: SOURCE,
    next,
    edit: singleEdit(SOURCE, next),
    oldBytes: new ByteIndex(SOURCE),
    newBytes: new ByteIndex(next),
  });
  assert.ok(ok, "the patch declined a patchable edit");

  // The store now names `next`'s elements, so a full compile against it must agree
  // with the patched tree node for node — same parse, same serialization, so any
  // stranded def, dropped comment, or misplaced splice shows as a diff.
  const expected = document.createElement("div");
  expected.innerHTML = compile(next, { sourcemap: true, keys: keys.key });
  assert.equal(container.innerHTML, expected.innerHTML);

  // And no stale duplicate ids — the failure stranded defs would actually cause.
  const ids = Array.from(container.querySelectorAll("[id]"), (el) => el.id);
  assert.equal(new Set(ids).size, ids.length, `duplicate ids after patch: ${ids}`);
});

check("an unconfinable edit declines and leaves the DOM untouched", () => {
  const { keys, container, session, svg } = render(SOURCE);
  const before = container.innerHTML;
  // Editing the root tag itself has no fragment unit to re-emit.
  const next = SOURCE.replace(`viewBox="0 0 400 300"`, `viewBox="0 0 400 3000"`);
  const ok = patchDocument({ session, keys, spans: nodeSpans }, svg, {
    compiled: SOURCE,
    next,
    edit: singleEdit(SOURCE, next),
    oldBytes: new ByteIndex(SOURCE),
    newBytes: new ByteIndex(next),
  });
  assert.equal(ok, false);
  assert.equal(container.innerHTML, before, "a declined patch must change nothing");
});

console.log(`dom-patch: ${passed} checks passed`);
