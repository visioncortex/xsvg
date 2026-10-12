#!/usr/bin/env node
// The outline seam's `advances` (GlyphOutliner::advances, docs/Incremental.md §7) through
// the node adapter: its one-pass answer must match the per-prefix definition exactly —
// entry i is where character i+1 begins, kerning kept; the last is the run's width — on
// the bundled fonts, and a compiled text-on-path effect must take the outlined route with
// it (an unanswered seam would silently degrade the effect to live text).
//
//   npm run check:advances
import assert from "node:assert/strict";
import { resolve } from "node:path";
import { compile } from "../packages/xsvg-compile/dist/index.js";
import { advancesOf, loadFontDir } from "../packages/xsvg-compile/dist/fonts.js";

const FONTS = resolve(import.meta.dirname, "../assets/fonts");
const fonts = loadFontDir(FONTS);
let checks = 0;
const ok = (cond, msg) => {
  assert.ok(cond, msg);
  checks++;
};
const near = (a, b) => Math.abs(a - b) < 1e-6;

for (const [family, style] of [["Anton", "normal"], ["Arimo", "normal"], ["Arimo", "italic"]]) {
  const font = fonts.resolve(family, style);
  ok(font, `${family} ${style} loads from assets/fonts`);
  const size = 24;
  for (const text of ["hey", "AVAW Tyf", "naïve café", "x"]) {
    const adv = advancesOf(font, text, size);
    const chars = Array.from(text);
    const tag = `${family} ${style} ${JSON.stringify(text)}`;
    assert.equal(adv.length, chars.length, `${tag}: one entry per character`);
    checks++;
    ok(near(adv[adv.length - 1], font.getAdvanceWidth(text, size)), `${tag}: the last entry is the run's width`);
    for (let i = 0; i + 1 < chars.length; i++) {
      // where character i+1 begins = the prefix through it, less its own advance
      const prefix = chars.slice(0, i + 2).join("");
      const own = (font.charToGlyph(chars[i + 1]).advanceWidth * size) / font.unitsPerEm;
      ok(near(adv[i], font.getAdvanceWidth(prefix, size) - own), `${tag}: entry ${i} is the kerned origin of character ${i + 1}`);
    }
    ok(adv.every((a) => Number.isFinite(a)), `${tag}: finite`);
  }
  assert.deepEqual(advancesOf(font, "", size), [], `${family}: an empty run answers []`);
  checks++;
}
assert.deepEqual(advancesOf(undefined, "hey", 24), [], "no font answers []");
checks++;

// end to end: with the fonts, skew and rainbow outline into paths; follow is live by design
const src = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 340 160">
<path id="p" d="M 20 80 C 120 0 220 160 320 80" fill="none"/>
<x:textpath in="#p" effect="skew" font-family="Anton" font-size="30">skewed along</x:textpath>
<x:textpath in="#p" effect="rainbow" font-family="Anton" font-size="30">over the rainbow</x:textpath>
<x:textpath in="#p" effect="follow" font-family="Anton" font-size="30">follow</x:textpath>
</svg>`;
const out = compile(src, { fontDir: FONTS });
assert.equal((out.match(/<textPath/g) ?? []).length, 1, "follow lowers to one <textPath>");
checks++;
assert.equal((out.match(/<text[\s>]/g) ?? []).length, 1, `skew and rainbow must outline, not degrade to live text:\n${out}`);
checks++;
ok((out.match(/<path[\s>]/g) ?? []).length === 3, "the reference path and two outlined runs");

console.log(`advances: ${checks} checks passed`);
