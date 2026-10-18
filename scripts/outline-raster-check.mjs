// Checks that outlined text draws where the live text drew (Specification.md
// §6.12; the `to_path` conversion an editor writes back, docs/Incremental.md §11).
//
// Create-outlines replaces live text by glyph paths traced at the compiler's own
// placement. That is a claim about rendering — "these paths are those glyphs" —
// so this renders: each case is compiled twice, live and with outlining asked
// for (`outline="true"` on a box, `x:outline="true"` on point text), drawn through
// resvg with the bundled fonts, and the two rasters compared.
//
// With the same font files, resvg's own text and our traced paths land on the
// same pixels (0 differ on every case at the time of writing; a traced list
// marker is the odd 30). The tolerance below is a tenth of one small glyph:
// a chunk at the wrong pen, an anchor over the wrong extent, a lost decoration
// or paragraph moves whole words and fails loudly.
//
// Each case is also taken the editor's way: `xsvg --to-path OFFSET` replaces the
// node in the SOURCE by its conversion, that document is compiled, and its raster
// must match the live one too — Convert to path leaves the picture as it was.
//
//   npm run check:outline       (needs `resvg` on PATH and a release CLI build)

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";

if (spawnSync("resvg", ["--version"]).status !== 0) {
  console.log("outline-raster: skipped — resvg not on PATH (cargo install resvg)");
  process.exit(0);
}

const tmp = mkdtempSync(join(tmpdir(), "xsvg-outline-"));
const ZOOM = 2;
const MAX_DIFF_FRACTION = 0.002;
const FONTS = "assets/fonts";

const page = (body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 400 200" width="400" height="200"><rect width="400" height="200" fill="#fff"/>${body}</svg>`;

// [name, live body, outlined body]
const cases = [
  ["box-single", (o) => `<x:textbox x="10" y="10" width="380" height="60" font-family="Arimo" font-size="28" fill="#123"${o}>Quick brown foxes</x:textbox>`],
  ["box-centred-rotated", (o) => `<x:textbox x="50" y="40" width="300" height="80" font-family="Arimo" font-size="24" align="center" valign="middle" transform="rotate(-8 200 80)"${o}>Rotated, centred</x:textbox>`],
  ["box-runs", (o) => `<x:textbox x="10" y="10" width="380" height="120" font-family="Arimo" font-size="22"${o}>plain <tspan fill="#c00">red</tspan> and <tspan x:highlight="#ff0">marked</tspan> and <tspan text-decoration="underline">under</tspan> words that wrap onto a second line</x:textbox>`],
  ["box-paragraphs", (o) => `<x:textbox x="10" y="10" width="380" height="180" font-family="Arimo" font-size="20"${o}><x:p>First paragraph here.</x:p><x:p font-size="26" fill="#06c">Second, larger.</x:p><x:list list="number"><x:li>one item</x:li><x:li indent="1">nested item</x:li></x:list></x:textbox>`],
  ["textarea", (o) => `<textArea x="10" y="10" width="380" font-family="Arimo" font-size="22" text-align="end"${o.replace('outline=', 'x:outline=')}>Area text, right aligned, wrapping to another line</textArea>`],
  ["point-start", (o) => `<text x="20" y="60" font-family="Arimo" font-size="30" fill="#123"${o.replace('outline=', 'x:outline=')}>Point text</text>`],
  ["point-anchors", (o) => `<text x="200" y="50" font-family="Arimo" font-size="26" text-anchor="middle"${o.replace('outline=', 'x:outline=')}>centred <tspan font-weight="700">bold</tspan></text><text x="380" y="120" font-family="Arimo" font-size="26" text-anchor="end"${o.replace('outline=', 'x:outline=')}>ends here</text>`],
  ["point-tspans", (o) => `<text x="20" y="60" font-family="Arimo" font-size="26"${o.replace('outline=', 'x:outline=')}>ab<tspan fill="#c00" dy="8">cd</tspan><tspan x="20" y="120" font-size="36">new line</tspan></text>`],
];

/** The source with every convertible element replaced by its `to_path` markup —
 *  what an editor's Convert to path writes — converted last to first so earlier
 *  offsets hold. */
function convertAll(source, name) {
  const starts = [...source.matchAll(/<(x:textbox|textArea|text)[\s>]/g)].map((m) => m.index).reverse();
  let doc = source;
  for (const [i, at] of starts.entries()) {
    const file = join(tmp, `${name}-src-${i}.xsvg`);
    writeFileSync(file, doc);
    doc = execFileSync("./target/release/xsvg", ["--font-directory", FONTS, "--to-path", String(at), file], { encoding: "utf8" });
  }
  return doc;
}

function render(source, name) {
  const file = join(tmp, `${name}.svg`);
  writeFileSync(file, source);
  execFileSync("resvg", ["--zoom", String(ZOOM), "--use-fonts-dir", FONTS, "--skip-system-fonts", file, join(tmp, `${name}.png`)]);
  return PNG.sync.read(readFileSync(join(tmp, `${name}.png`)));
}
function compile(source, name) {
  const file = join(tmp, `${name}.xsvg`);
  writeFileSync(file, source);
  return execFileSync("./target/release/xsvg", ["--font-directory", FONTS, file], { encoding: "utf8" });
}

let failed = 0;
for (const [name, body] of cases) {
  const live = compile(page(body("")), `${name}-live`);
  const outlined = compile(page(body(' outline="true"')), `${name}-outlined`);
  if (/<text[\s>]/.test(outlined)) {
    console.log(`✗ ${name}: still live text after outlining — a line could not be traced`);
    failed++;
    continue;
  }
  // the editor's path: to_path, the node replaced in the SOURCE, compiled again
  const replaced = compile(convertAll(page(body("")), name), `${name}-replaced`);
  if (/<text[\s>]/.test(replaced)) {
    console.log(`✗ ${name}: live text left after to_path replacement`);
    failed++;
    continue;
  }
  const a = render(live, `${name}-live`);
  const compare = (what, source) => {
    const c = render(source, `${name}-${what}`);
    const diff = new PNG({ width: a.width, height: a.height });
    const n = pixelmatch(a.data, c.data, diff.data, a.width, a.height, { threshold: 0.3 });
    const frac = n / (a.width * a.height);
    const ok = frac <= MAX_DIFF_FRACTION;
    if (!ok) {
      writeFileSync(join(tmp, `${name}-${what}-diff.png`), PNG.sync.write(diff));
      failed++;
    }
    console.log(`${ok ? "✓" : "✗"} ${name} (${what}): ${n} px differ (${(frac * 100).toFixed(2)}%)${ok ? "" : ` — see ${tmp}/${name}-${what}-diff.png`}`);
  };
  compare("outlined", outlined);
  compare("to_path+replace", replaced);
}
if (failed) {
  console.log(`outline-raster: ${failed} case(s) differ`);
  process.exit(1);
}
console.log(`outline-raster: ${cases.length} cases outline where they drew`);
