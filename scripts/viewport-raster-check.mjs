// Checks that the nested-viewport lowering (Specification.md §5.3) draws the same
// pixels as the viewport it replaces.
//
// A nested <svg> is the authoring form of a viewport; the compiler lowers it to one
// clipped <g> with the frame and the viewBox fit folded into its transform. That is
// a claim about rendering — "a <g> with this transform and this clip is this
// viewport" — so this renders: every case below is drawn twice through resvg, once
// as authored (plain SVG, which resvg draws per the spec, nested viewports and
// their transforms included) and once compiled, and the two rasters must match.
// The whole-file link case has no plain-SVG twin, so its twin is hand-written as
// the nested <svg> the old lowering emitted.
//
// Engines disagree about a filter on a nested <svg> (WebKit drops it, Chromium loses
// the clip), so that case is not compared against the authored form; the
// structure it lowers to is pinned by the core's unit tests.
//
//   npm run check:viewport      (needs `resvg` on PATH and a release CLI build)

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";

if (spawnSync("resvg", ["--version"]).status !== 0) {
  console.log("viewport-raster: skipped — resvg not on PATH (cargo install resvg)");
  process.exit(0);
}

const tmp = mkdtempSync(join(tmpdir(), "xsvg-viewport-"));
const ZOOM = 3;
// antialiased edges may differ by a pixel where a clip edge and a shape edge are
// rasterised in different orders; anything past this is a different drawing
const MAX_DIFF_FRACTION = 0.002;

// a 2×2 picture — red, green / blue, yellow — so a crop's visible fraction is legible
const PNG2 = (() => {
  const png = new PNG({ width: 2, height: 2 });
  const px = [[220, 40, 40], [40, 160, 60], [40, 80, 220], [240, 200, 40]];
  px.forEach(([r, g, b], i) => { png.data[i * 4] = r; png.data[i * 4 + 1] = g; png.data[i * 4 + 2] = b; png.data[i * 4 + 3] = 255; });
  return "data:image/png;base64," + PNG.sync.write(png).toString("base64");
})();
const unit = `<image x="0" y="0" width="1" height="1" preserveAspectRatio="none" image-rendering="pixelated" href="${PNG2}"/>`;
const wide = `<rect width="200" height="100" fill="#48a"/><circle cx="200" cy="50" r="30" fill="#fff"/><rect x="0" y="80" width="40" height="40" fill="#e11"/>`;

const page = (body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 300 200" width="300" height="200"><rect width="300" height="200" fill="#f1f5f9"/>${body}</svg>`;

const cases = {
  "crop-none-rotated": `<svg x="20" y="20" width="120" height="80" viewBox="0.5 0 0.5 1" preserveAspectRatio="none" transform="rotate(15 80 60)">${unit}</svg>`,
  "crop-offset-viewbox": `<svg x="20" y="20" width="120" height="80" viewBox="0.25 0.25 0.5 0.5" preserveAspectRatio="none">${unit}</svg>`,
  "meet-default": `<svg x="100" y="20" width="100" height="100" viewBox="0 0 200 100">${wide}</svg>`,
  "meet-xminymax": `<svg x="100" y="20" width="100" height="100" viewBox="0 0 200 100" preserveAspectRatio="xMinYMax meet">${wide}</svg>`,
  "slice-xmaxymin": `<svg x="100" y="20" width="100" height="100" viewBox="0 0 200 100" preserveAspectRatio="xMaxYMin slice">${wide}</svg>`,
  "slice-default": `<svg x="100" y="20" width="100" height="100" viewBox="0 0 200 100" preserveAspectRatio="xMidYMid slice" transform="translate(50 20) rotate(-10)">${wide}</svg>`,
  "no-viewbox-clips": `<svg x="20" y="20" width="60" height="40">${wide}</svg>`,
  "overflow-visible": `<svg x="20" y="20" width="60" height="40" viewBox="0 0 200 100" overflow="visible">${wide}</svg>`,
  "percent-and-nested": `<svg x="10%" y="10%" width="50%" height="20%">${wide}</svg><svg x="150" y="100" width="100" height="100" viewBox="0 0 10 10"><rect width="10" height="10" fill="#cde"/><svg x="1" y="1" width="50%" height="50%" viewBox="0 0 200 100">${wide}</svg></svg>`,
  "opacity-and-paint": `<svg x="20" y="20" width="100" height="100" viewBox="0 0 200 100" opacity="0.5" fill="#0a0"><rect width="200" height="100"/><rect x="50" y="20" width="100" height="60" fill="#00f"/></svg>`,
  "use-sized-kept": `<svg id="icon" x="20" y="20" width="24" height="24" viewBox="0 0 100 100" transform="rotate(20 32 32)"><circle cx="50" cy="50" r="50" fill="#48a"/><rect width="50" height="50" fill="#e11"/></svg><use href="#icon" x="150" y="50" width="96" height="96"/>`,
};

// the whole-file link: the <use> compiles through the lowering; its twin is the
// nested <svg> the old form emitted — the browser's own viewport fit
writeFileSync(join(tmp, "logo.xsvg"), `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100">${wide}</svg>`);
const linkCases = {
  "link-meet": [`<use href="logo.xsvg" x="20" y="20" width="100" height="100" transform="rotate(5 70 70)" opacity="0.8"/>`,
    `<svg x="20" y="20" width="100" height="100" viewBox="0 0 200 100" transform="rotate(5 70 70)" opacity="0.8">${wide}</svg>`],
  "link-none": [`<use href="logo.xsvg" x="20" y="20" width="250" height="60" preserveAspectRatio="none"/>`,
    `<svg x="20" y="20" width="250" height="60" viewBox="0 0 200 100" preserveAspectRatio="none">${wide}</svg>`],
};

function render(source, name) {
  const file = join(tmp, `${name}.svg`);
  writeFileSync(file, source);
  execFileSync("resvg", ["--zoom", String(ZOOM), file, join(tmp, `${name}.png`)]);
  return PNG.sync.read(readFileSync(join(tmp, `${name}.png`)));
}
function compile(source, name) {
  const file = join(tmp, `${name}.xsvg`);
  writeFileSync(file, source);
  return execFileSync("./target/release/xsvg", [file], { encoding: "utf8" });
}

let failed = 0;
const check = (name, authored, compiled) => {
  // the root, plus the one viewport a <use> sizes (its marker comment names <svg> too)
  const elements = (compiled.replace(/<!--[\s\S]*?-->/g, "").match(/<svg[\s>]/g) ?? []).length;
  if (elements !== (name === "use-sized-kept" ? 2 : 1)) {
    console.log(`✗ ${name}: unexpected nested <svg> in compiled output`);
    failed++;
    return;
  }
  const a = render(authored, `${name}-authored`);
  const c = render(compiled, `${name}-compiled`);
  const diff = new PNG({ width: a.width, height: a.height });
  const n = pixelmatch(a.data, c.data, diff.data, a.width, a.height, { threshold: 0.1 });
  const frac = n / (a.width * a.height);
  const ok = frac <= MAX_DIFF_FRACTION;
  if (!ok) {
    writeFileSync(join(tmp, `${name}-diff.png`), PNG.sync.write(diff));
    failed++;
  }
  console.log(`${ok ? "✓" : "✗"} ${name}: ${n} px differ (${(frac * 100).toFixed(3)}%)${ok ? "" : ` — see ${tmp}/${name}-diff.png`}`);
};

for (const [name, body] of Object.entries(cases)) {
  const authored = page(body);
  check(name, authored, compile(authored, name));
}
for (const [name, [use, twin]] of Object.entries(linkCases)) {
  check(name, page(twin), compile(page(use), name));
}

if (failed) {
  console.log(`viewport-raster: ${failed} case(s) differ`);
  process.exit(1);
}
console.log(`viewport-raster: ${Object.keys(cases).length + Object.keys(linkCases).length} viewports draw as authored`);
