// Checks the source surgery behind the inspector's parameter controls
// (packages/xsvg-viewer/src/params.ts).
//
// These functions rewrite the author's own document in place, so a bug here
// corrupts source rather than merely rendering wrong — worth pinning even though
// the repo has no JS test runner. Run with:
//
//   node --experimental-strip-types scripts/params-check.mjs

import assert from "node:assert/strict";
import { namespaceOf, parseStartTag, setAttribute } from "../packages/xsvg-viewer/src/params.ts";

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

if (!process.exitCode) console.log(`params: ${passed} checks passed`);
