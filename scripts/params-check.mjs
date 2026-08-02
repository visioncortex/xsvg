// Checks the source surgery behind the inspector's parameter controls
// (packages/xsvg-viewer/src/params.ts).
//
// These functions rewrite the author's own document in place, so a bug here
// corrupts source rather than merely rendering wrong — worth pinning even though
// the repo has no JS test runner. Run with:
//
//   node --experimental-strip-types scripts/params-check.mjs

import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  namespaceOf,
  parseStartTag,
  prefixOf,
  renderParams,
  setAttribute,
} from "../packages/xsvg-viewer/src/params.ts";
import { createInspector } from "../packages/xsvg-viewer/src/inspector.ts";
import { NodeKeyStore } from "../packages/xsvg-viewer/src/keys.ts";

// renderParams builds DOM, exactly as it does in the browser; give it jsdom's.
globalThis.document = new JSDOM("<!doctype html><body></body>").window.document;

let passed = 0;
function check(name, fn) {
  try {
    fn();
    passed++;
  } catch (e) {
    console.error(`FAIL  ${name}\n      ${e.message}`);
    process.exitCode = 1;
  }
}

check("parses a qualified start tag with its attributes", () => {
  const tag = parseStartTag(`<clip:arrow x1="30" y1="60" fill="var(accent)"/>`);
  assert.equal(tag.name, "clip:arrow");
  assert.equal(tag.prefix, "clip");
  assert.equal(tag.local, "arrow");
  assert.equal(tag.attrs.get("x1").value, "30");
  assert.equal(tag.attrs.get("fill").value, "var(accent)");
});

check("survives the shapes real documents take", () => {
  // no prefix, single quotes, newlines between attributes, a bare `>` close
  const tag = parseStartTag(`<arrow\n  a='1'\n  b = "2"\n>`);
  assert.equal(tag.prefix, "");
  assert.equal(tag.attrs.get("a").value, "1");
  assert.equal(tag.attrs.get("b").value, "2");
  // an attribute value containing > or / must not end the tag early
  const tricky = parseStartTag(`<g d="M0 0 L1 1" c="a/b>c" x="9"/>`);
  assert.equal(tricky.attrs.get("c").value, "a/b>c");
  assert.equal(tricky.attrs.get("x").value, "9");
});

check("refuses malformed input instead of guessing", () => {
  assert.equal(parseStartTag("not a tag"), null);
  assert.equal(parseStartTag("<unterminated a=\"1\""), null);
  assert.equal(parseStartTag("<a b=unquoted/>"), null);
});

check("resolves a prefix to its namespace", () => {
  const src = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:clip="https://x.example/c">`;
  assert.equal(namespaceOf(src, "clip"), "https://x.example/c");
  assert.equal(namespaceOf(src, ""), "http://www.w3.org/2000/svg");
  assert.equal(namespaceOf(src, "nope"), null);
});

check("replaces an existing attribute in place", () => {
  const src = `<a/>\n<clip:arrow x1="30" thickness="22"/>`;
  const at = src.indexOf("<clip:arrow");
  const out = setAttribute(src, at, "thickness", "40");
  assert.equal(out, `<a/>\n<clip:arrow x1="30" thickness="40"/>`);
  // everything outside the one attribute is untouched, including formatting
  assert.equal(out.slice(0, at), src.slice(0, at));
});

check("inserts an attribute that was not there", () => {
  const src = `<clip:arrow x1="30"/>`;
  assert.equal(setAttribute(src, 0, "radius", "4"), `<clip:arrow x1="30" radius="4"/>`);
  // and on a tag closed with a plain `>`
  assert.equal(setAttribute(`<clip:box a="1">`, 0, "b", "2"), `<clip:box a="1" b="2">`);
});

check("an empty value clears the attribute, falling back to the module default", () => {
  assert.equal(setAttribute(`<clip:arrow x1="30" radius="4"/>`, 0, "radius", ""), `<clip:arrow x1="30"/>`);
  // clearing one that is not set is a no-op, not a stray space
  assert.equal(setAttribute(`<clip:arrow x1="30"/>`, 0, "radius", ""), `<clip:arrow x1="30"/>`);
});

check("escapes values so a parameter cannot break out of the attribute", () => {
  const out = setAttribute(`<clip:arrow/>`, 0, "fill", `"><script>alert(1)</script>`);
  assert.equal(out, `<clip:arrow fill="&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;"/>`);
  // and the result still parses as the same single element
  assert.equal(parseStartTag(out).attrs.get("fill").value, "&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;");
});

check("multi-byte text before the tag does not shift the edit", () => {
  const src = `<t>héllo → 世界 🌍</t><clip:arrow x1="1"/>`;
  const at = src.indexOf("<clip:arrow");
  assert.equal(setAttribute(src, at, "x1", "2"), `<t>héllo → 世界 🌍</t><clip:arrow x1="2"/>`);
});

// ---- what the panel offers ------------------------------------------------

const XSVG_NS = "https://xsvg.visioncortex.org";
const DOC = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="${XSVG_NS}">`;

/** The controls rendered for a spec, as `name → the element that carries value`. */
function offered(spec, tagSource, source = DOC) {
  const tag = parseStartTag(tagSource);
  const wrap = renderParams(spec, tag.attrs, () => {}, (ns) => prefixOf(source, ns));
  const out = new Map();
  for (const row of wrap.querySelectorAll(".xsvg-param")) {
    out.set(row.querySelector(".xsvg-param-name").textContent, row);
  }
  return { names: [...out.keys()], rows: out, text: wrap.textContent };
}

check("resolves a namespace to the prefix this document binds", () => {
  assert.equal(prefixOf(DOC, XSVG_NS), "x");
  assert.equal(prefixOf(`<svg xmlns:xsvg="${XSVG_NS}">`, XSVG_NS), "xsvg");
  // a default xmlns binds elements, not attributes — it cannot spell one
  assert.equal(prefixOf(`<svg xmlns="${XSVG_NS}">`, XSVG_NS), null);
  assert.equal(prefixOf(DOC, "https://other.example/"), null);
});

check("geometry is not offered as a control", () => {
  const spec = {
    params: {
      x: { type: "number", geometry: true },
      width: { type: "number", min: 0, max: 100, geometry: true },
      fill: { type: "color" },
      "stroke-width": { type: "number", min: 0, max: 40 },
    },
  };
  const { names } = offered(spec, `<x:pie x="10" width="80" fill="#48a"/>`);
  // position and extent belong to the canvas and the source pane; what is left is
  // the styling half, which is the whole point of the panel
  assert.deepEqual(names, ["fill", "stroke-width"]);
});

check("an element that is all geometry says so instead of showing a blank block", () => {
  const spec = { params: { x: { type: "number", geometry: true } } };
  const { names, text } = offered(spec, `<x:thing x="1"/>`);
  assert.deepEqual(names, []);
  assert.match(text, /only geometry/);
});

check("a namespaced attribute takes the document's own prefix", () => {
  const spec = { params: { "border-width": { type: "number", min: 0, max: 100, ns: XSVG_NS } } };

  // labelled and read under the prefix this document binds...
  const bound = offered(spec, `<x:textbox x:border-width="4"/>`);
  assert.deepEqual(bound.names, ["x:border-width"]);
  assert.equal(bound.rows.get("x:border-width").querySelector("input").value, "4");

  // ...which is whatever the document chose, not a hard-coded `x`
  const other = `<svg xmlns:xs="${XSVG_NS}">`;
  const renamed = offered(spec, `<xs:textbox xs:border-width="7"/>`, other);
  assert.deepEqual(renamed.names, ["xs:border-width"]);
  assert.equal(renamed.rows.get("xs:border-width").querySelector("input").value, "7");

  // and is skipped outright when the document binds no prefix for it: writing an
  // unbound prefix would not merely be wrong, it would be malformed XML
  const unbound = offered(spec, `<textbox/>`, `<svg xmlns="http://www.w3.org/2000/svg">`);
  assert.deepEqual(unbound.names, []);
});

check("editing a namespaced attribute commits under its qualified name", () => {
  const spec = { params: { "border-color": { type: "color", ns: XSVG_NS } } };
  const tag = parseStartTag(`<x:textbox/>`);
  const commits = [];
  const wrap = renderParams(spec, tag.attrs, (n, v) => commits.push([n, v]), (ns) => prefixOf(DOC, ns));
  const input = wrap.querySelector("input");
  input.value = "#111";
  input.dispatchEvent(new globalThis.document.defaultView.Event("change"));
  assert.deepEqual(commits, [["x:border-color", "#111"]]);
  // and that qualified name is what the source surgery writes
  assert.equal(
    setAttribute(`<x:textbox/>`, 0, "x:border-color", "#111"),
    `<x:textbox x:border-color="#111"/>`,
  );
});

// ---- the panel survives an element it edited into invisibility ------------

const SVG_NS = "http://www.w3.org/2000/svg";

/** A pinned `<clip:arrow>` and the levers to make it vanish, wired the way the
 *  interactive viewer wires them. */
function pinnedArrow(attrs = 'thickness="8"', rendered = true) {
  const source = `<svg xmlns:clip="https://x.example/c"><clip:arrow ${attrs}/></svg>`;
  const start = source.indexOf("<clip:arrow");
  const end = source.indexOf("/>", start) + 2;

  const svgRoot = document.createElementNS(SVG_NS, "svg");
  const el = document.createElementNS(SVG_NS, "path");
  // the store mints names in document order, so this element is `n0`
  el.setAttribute("data-xsvg-node", "n0");
  // A compiled shape carries the geometry the generator emitted, not the author's
  // attributes — and that `d` is routinely enormous.
  el.setAttribute("d", `M0,0 ${"L1,1 ".repeat(400)}Z`);
  // `rendered: false` is the state after a recompile of a document whose generator
  // emitted only a marker: the element is in the source, and in no DOM at all.
  if (rendered) svgRoot.append(el);

  const keys = new NodeKeyStore();
  keys.sync([[start, end]]);

  const panel = document.createElement("div");
  const edits = [];
  const inspector = createInspector({
    svgRoot,
    panel,
    source,
    editor: { highlight() {} },
    schemas: { spec: () => ({ params: { thickness: { type: "number", min: 0, max: 200 } } }) },
    index: keys,
    onEdit: (next, at) => edits.push([next, at]),
  });
  return { inspector, panel, svgRoot, el, source, start, edits };
}

/** Fire the panel's thickness control, as a user dragging the slider does. */
function drag(panel, value) {
  const range = panel.querySelector('input[type=range]');
  assert.ok(range, "no thickness slider in the panel");
  range.value = String(value);
  range.dispatchEvent(new panel.ownerDocument.defaultView.Event("input"));
}

check("an element edited into invisibility can still be edited back", () => {
  // <clip:arrow thickness="0"> has no area, so the generator degrades it to a marker
  // comment and the element leaves the drawing entirely. The author's next move is to
  // undo that — but the canvas now offers nothing to click, so the panel is the only
  // way back. Dropping the pin when the element vanished left the controls standing
  // and inert: visibly live, silently doing nothing.
  const { inspector, panel, el, source, start, edits } = pinnedArrow();
  inspector.selectAtSourceOffset(start + 2);
  assert.ok(panel.querySelector("input[type=range]"), "the arrow did not pin");
  assert.ok(panel.querySelector("table.attrs"), "should pin the rendered element first");

  // thickness=0 → the generator emits only a marker, so nothing is rendered for it
  const gone = source.replace('thickness="8"', 'thickness="0"');
  el.remove();
  inspector.refresh(gone);

  assert.match(panel.textContent, /renders nothing/, "the panel should say why it is empty");
  drag(panel, 12);
  assert.equal(edits.length, 1, "the control is dead — the author cannot undo the edit");
  assert.match(edits[0][0], /thickness="12"/);
  // and it edits the right element: the range, not a stale DOM node
  assert.equal(edits[0][1], gone.indexOf("<clip:arrow"));
});

check("a re-pin after a full recompile finds an element that renders nothing", () => {
  // The patch path declines when a fragment has no anchor, so the viewer recompiles
  // and re-pins by source offset. Nothing rendered covers that offset any more, so
  // without the source-span fallback the panel would come back dead.
  const { inspector, panel, start, edits } = pinnedArrow('thickness="0"', false);
  inspector.selectAtSourceOffset(start + 2);

  assert.match(panel.textContent, /renders nothing/);
  drag(panel, 9);
  assert.equal(edits.length, 1, "re-pinning an unrendered element left the panel dead");
});

check("a long attribute value is elided rather than flooding the panel", () => {
  const { inspector, panel, start } = pinnedArrow();
  inspector.selectAtSourceOffset(start + 2);
  const d = panel.querySelector("table.attrs td[title]");
  assert.ok(d, "a 2000-char d should be elided and carry the full value as a tooltip");
  assert.ok(d.textContent.includes("…"), `not elided: ${d.textContent.slice(0, 40)}`);
  assert.match(d.textContent, /\d[\d,]* chars/, "should say how much was hidden");
  assert.ok(d.textContent.length < 140, `still too long: ${d.textContent.length}`);
  // the head survives — it is what identifies the value
  assert.ok(d.textContent.startsWith("M0,0 L1,1"), d.textContent.slice(0, 20));
});

if (!process.exitCode) console.log(`params: ${passed} checks passed`);
