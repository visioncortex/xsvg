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
  ["box-bold-italic", (o) => `<x:textbox x="10" y="10" width="380" height="120" font-family="Arimo" font-size="24"${o}>regular <tspan font-weight="700">bold words</tspan> then <tspan font-style="italic">italic ones</tspan> and <tspan font-size="34">larger</tspan> to wrap the line</x:textbox>`],
  ["box-tracking", (o) => `<x:textbox x="10" y="10" width="380" height="120" font-family="Arimo" font-size="22" letter-spacing="3" word-spacing="8"${o}>Tracked words, spaced out, wrapping to a second line here</x:textbox>`],
  ["box-justified", (o) => `<x:textbox x="10" y="10" width="300" height="160" font-family="Arimo" font-size="20" align="justify"${o}>Justified text spreads its words to both edges of the box on every full line but the last one</x:textbox>`],
  ["box-glyph-scale", (o) => `<x:textbox x="10" y="10" width="380" height="60" font-family="Arimo" font-size="24" glyph-x-scale="1.5"${o}>Wide glyphs here</x:textbox>`],
  // curved shapes convert to cubics: the editor's leg only (nothing to outline)
  ["shapes-curved", (o) => `<rect x="20" y="20" width="160" height="90" rx="22" fill="#48a"${o && ""}/><ellipse cx="300" cy="65" rx="80" ry="40" fill="#e11"/><circle cx="100" cy="160" r="30" fill="#0a0"/><rect x="200" y="130" width="60" height="60" rx="30" fill="#06c" stroke="#000" stroke-width="3"/>`],
  // an x:boolean converts on its curves (Incremental.md §11): the replaced document renders the same
  ["shapes-boolean", (o) => `<x:boolean op="union" fill="#48a"${o}><rect x="20" y="20" width="200" height="120" rx="24"/><circle cx="230" cy="80" r="60"/></x:boolean>`],
  ["shapes-boolean-holes", (o) => `<x:boolean op="subtract" fill="#262"${o}><rect x="20" y="20" width="360" height="160" rx="30"/><circle cx="120" cy="100" r="50"/><circle cx="280" cy="100" r="50"/><polygon points="180,40 220,160 160,160 240,40"/></x:boolean>`],
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
  const starts = [...source.matchAll(/<(x:boolean|x:textbox|textArea|text|rect|ellipse|circle)[\s>]/g)].map((m) => m.index).reverse();
  let doc = source;
  for (const [i, at] of starts.entries()) {
    const file = join(tmp, `${name}-src-${i}.xsvg`);
    writeFileSync(file, doc);
    doc = execFileSync("./target/release/xsvg", ["--font-directory", FONTS, "--to-path", String(at), file], { encoding: "utf8" });
  }
  return doc;
}

// [name, body, twin]: two authored forms that must draw the same pixels — a
// shape with x:corners against the rounded shape it describes
const twinCases = [
  ["corners-rect", `<rect x="20" y="20" width="200" height="120" x:corners="24" fill="#48a"/>`, `<rect x="20" y="20" width="200" height="120" rx="24" fill="#48a"/>`],
  ["corners-token", `<x:theme><x:color name="radius" value="16"/></x:theme><rect x="20" y="20" width="200" height="120" x:corners="var(radius)" fill="#e11" stroke="#000" stroke-width="4"/>`, `<rect x="20" y="20" width="200" height="120" rx="16" fill="#e11" stroke="#000" stroke-width="4"/>`],
  ["corners-stadium", `<rect x="20" y="20" width="200" height="80" x:corners="100" fill="#0a0"/>`, `<rect x="20" y="20" width="200" height="80" rx="40" fill="#0a0"/>`],
  // one corner at the side length is a quarter circle: the circle clipped to the square
  ["corners-quarter", `<rect x="20" y="20" width="160" height="160" x:corners="0:160" fill="#48a"/>`, `<clipPath id="q"><rect x="20" y="20" width="160" height="160"/></clipPath><circle cx="180" cy="180" r="160" clip-path="url(#q)" fill="#48a"/>`],
  // a chamfer is the polygon through the trim points
  ["corners-chamfer", `<rect x="20" y="20" width="200" height="120" x:corners="24" x:corner-style="chamfer" fill="#a40"/>`, `<polygon points="44,20 196,20 220,44 220,116 196,140 44,140 20,116 20,44" fill="#a40"/>`],
  // an inverted 90° corner is the quarter disc about the apex cut away
  ["corners-scallop", `<rect x="20" y="20" width="200" height="120" x:corners="30" x:corner-style="inverted" fill="#808"/>`, `<mask id="m"><rect x="20" y="20" width="200" height="120" fill="#fff"/><circle cx="20" cy="20" r="30"/><circle cx="220" cy="20" r="30"/><circle cx="220" cy="140" r="30"/><circle cx="20" cy="140" r="30"/></mask><rect width="400" height="200" fill="#808" mask="url(#m)"/>`],
  // the winding does not matter
  ["corners-ccw", `<polygon points="20,20 220,20 220,140 20,140" x:corners="24" fill="#246"/>`, `<polygon points="20,20 20,140 220,140 220,20" x:corners="24" fill="#246"/>`],
  // a concave corner: the L plus the square at its inner corner, less the disc there
  ["corners-concave", `<path d="M20 20 H220 V80 H120 V140 H20 Z" x:corners="3:30" fill="#062"/>`, `<mask id="c"><path d="M20 20 H220 V80 H120 V140 H20 Z" fill="#fff"/><rect x="120" y="80" width="30" height="30" fill="#fff"/><circle cx="150" cy="110" r="30"/></mask><rect width="400" height="200" fill="#062" mask="url(#c)"/>`],
];

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
  const shapesOnly = name.startsWith("shapes-");
  if (!shapesOnly && /<text[\s>]/.test(outlined)) {
    console.log(`✗ ${name}: still live text after outlining — a line could not be traced`);
    failed++;
    continue;
  }
  // the editor's path: to_path, the node replaced in the SOURCE, compiled again
  const replaced = compile(convertAll(page(body("")), name), `${name}-replaced`);
  if (shapesOnly && /<(rect|ellipse|circle)[\s>]/.test(replaced.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<rect width="400" height="200" fill="#fff"\/>/, ""))) {
    console.log(`✗ ${name}: a shape left unconverted`);
    failed++;
    continue;
  }
  if (!shapesOnly && /<text[\s>]/.test(replaced)) {
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
  if (!shapesOnly) compare("outlined", outlined);
  compare("to_path+replace", replaced);
}
for (const [name, body, twin] of twinCases) {
  const a = render(compile(page(body), `${name}-a`), `${name}-a`);
  const c = render(compile(page(twin), `${name}-b`), `${name}-b`);
  const diff = new PNG({ width: a.width, height: a.height });
  const n = pixelmatch(a.data, c.data, diff.data, a.width, a.height, { threshold: 0.3 });
  const frac = n / (a.width * a.height);
  const ok = frac <= MAX_DIFF_FRACTION;
  if (!ok) { writeFileSync(join(tmp, `${name}-diff.png`), PNG.sync.write(diff)); failed++; }
  console.log(`${ok ? "✓" : "✗"} ${name} (twin): ${n} px differ (${(frac * 100).toFixed(2)}%)${ok ? "" : ` — see ${tmp}/${name}-diff.png`}`);
}
if (failed) {
  console.log(`outline-raster: ${failed} case(s) differ`);
  process.exit(1);
}
console.log(`outline-raster: ${cases.length} cases outline where they drew, ${twinCases.length} corner twins match`);
