#!/usr/bin/env node
// Compare two measurement dumps: per font, the width disagreement over every string
// and size (relative, so sizes compare), and the vertical metrics as a fraction of
// the em. Prints a table sorted by the worst width disagreement.
//
//   node scripts/font-measure/diff.mjs edge.json safari.json
import { readFileSync } from "node:fs";

const [aPath, bPath] = process.argv.slice(2);
const A = JSON.parse(readFileSync(aPath, "utf8"));
const B = JSON.parse(readFileSync(bPath, "utf8"));
console.log(`A: ${A.ua}\nB: ${B.ua}\n`);

const byFamily = new Map();
for (const [css, a] of Object.entries(A.fonts)) {
  const b = B.fonts[css];
  if (!b) continue;
  const size = Number(css.match(/(\d+)px/)[1]);
  const family = css.replace(/^normal (normal|bold) \d+px /, "");
  const weight = css.includes(" bold ") ? "bold" : "normal";
  const rec = byFamily.get(family) ?? { n: 0, maxRel: 0, maxKey: "", sumRel: 0, cnt: 0, vert: [], applied: new Set() };
  for (const [k, wa] of Object.entries(a.widths)) {
    const wb = b.widths[k];
    const ref = Math.max(wa, wb, 1e-9);
    const rel = Math.abs(wa - wb) / ref;
    rec.sumRel += rel; rec.cnt++;
    if (rel > rec.maxRel) { rec.maxRel = rel; rec.maxKey = `${weight} ${size}px "${k}": ${wa.toFixed(2)} vs ${wb.toFixed(2)}`; }
  }
  for (const m of ["ascent", "descent", "cap", "xh"]) {
    rec.vert.push([m, (a[m] - b[m]) / size]);
  }
  if (a.applied !== b.applied) rec.applied.add(`${a.applied} | ${b.applied}`);
  byFamily.set(family, rec);
}

const rows = [...byFamily.entries()].sort((x, y) => y[1].maxRel - x[1].maxRel);
console.log("family".padEnd(40), "mean width Δ".padStart(13), "max width Δ".padStart(12), "  worst case");
for (const [family, r] of rows) {
  console.log(family.padEnd(40), `${(100 * r.sumRel / r.cnt).toFixed(2)}%`.padStart(13), `${(100 * r.maxRel).toFixed(2)}%`.padStart(12), "  " + r.maxKey);
}
console.log("\nvertical metrics, A − B as a fraction of the em (max |Δ| per family):");
for (const [family, r] of rows) {
  const worst = {};
  for (const [m, d] of r.vert) worst[m] = Math.max(worst[m] ?? 0, Math.abs(d));
  console.log(family.padEnd(40), Object.entries(worst).map(([m, d]) => `${m} ${(100 * d).toFixed(1)}%`).join("  "));
}
const resolved = rows.filter(([, r]) => r.applied.size);
if (resolved.length) {
  console.log("\nshorthand resolved differently (A | B):");
  for (const [family, r] of resolved) for (const s of r.applied) console.log(`  ${family}: ${s}`);
}
