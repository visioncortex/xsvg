#!/usr/bin/env node
// Compare the native (xsvg CLI) compiler against the browser (wasm) compiler.
//
// For each dataset sample:
//   browser SVG = extracted from Edge --dump-dom of the dev-server preview page
//                 (the wasm compiler, real browser fonts) — the reference
//   native  SVG = ./target/release/xsvg dataset/<sample>.xsvg
//   structural  = element-tag sequence with <text> subtrees collapsed, so it is
//                 font-independent: catches geometry/structure divergence, ignores
//                 how many <tspan> lines the wrapping produced
//   pixel       = render both SVGs in Edge at the viewBox size, pixelmatch the PNGs
//
// Usage:  node scripts/compare.mjs [sample ...]      (default: every dataset sample
//         except the multi-artboard deck). Requires `npm run dev` running + Edge.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";

const EDGE = "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
const CLI = "./target/release/xsvg";
const SERVER = "http://localhost:5173";
const RENDER_MAX = 800; // longest rendered edge, px
const PIXEL_WARN = 1.0; // % differing pixels: below = good
const PIXEL_FAIL = 5.0; // above = investigate
const tmp = mkdtempSync(join(tmpdir(), "xsvg-cmp-"));

const args = process.argv.slice(2);
const samples = (
  args.length
    ? args
    : readdirSync("dataset")
        .filter((f) => f.endsWith(".xsvg"))
        .filter((f) => f !== "artboards.xsvg")
        // Excluded: cross-file <use> links resolve deps differently per surface (bundled
        // in-browser vs on-disk in the CLI), so it isn't a like-for-like parity target.
        .filter((f) => f !== "use-link.xsvg")
).map((s) => s.replace(/\.xsvg$/, ""));

function edge(extra, wantBuffer = false) {
  return execFileSync(
    EDGE,
    ["--headless=new", "--disable-gpu", "--hide-scrollbars", ...extra],
    { encoding: wantBuffer ? "buffer" : "utf8", maxBuffer: 128 << 20, stdio: ["ignore", "pipe", "ignore"] },
  );
}

// Browser-compiled SVG: dump the preview DOM and slice out the <svg> under #app.
function browserSvg(sample) {
  const dom = edge(["--virtual-time-budget=6000", "--dump-dom", `${SERVER}/preview/?file=${sample}.xsvg`]);
  const anchor = dom.indexOf('id="app"');
  const start = dom.indexOf("<svg", anchor);
  const end = dom.indexOf("</svg>", start);
  if (start < 0 || end < 0) throw new Error("no <svg> under #app");
  return dom.slice(start, end + 6);
}

function nativeSvg(sample) {
  return execFileSync(CLI, ["--font-directory", "assets/fonts", `dataset/${sample}.xsvg`], {
    encoding: "utf8",
    maxBuffer: 128 << 20,
  });
}

// Font-independent structural signature: the sequence of element tags, but a <text>
// element collapses to a single "text" token (its tspan children — driven by font
// metrics — are skipped). Divergence here means the compiler LOGIC differs.
function structure(svg) {
  const sig = [];
  const re = /<(\/?)([a-zA-Z][\w:-]*)([^>]*?)(\/?)>/g;
  const stack = [];
  let skipDepth = -1;
  let m;
  while ((m = re.exec(svg))) {
    const isClose = m[1] === "/";
    const tag = m[2];
    const selfClose = m[4] === "/";
    if (isClose) {
      stack.pop();
      if (skipDepth >= 0 && stack.length < skipDepth) skipDepth = -1;
      continue;
    }
    const skipping = skipDepth >= 0;
    if (!selfClose) stack.push(tag);
    if (skipping) continue;
    sig.push(tag);
    if (tag === "text" && !selfClose) skipDepth = stack.length;
  }
  return sig;
}

function structDiff(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) if (a[i] !== b[i]) return `@${i}: native=${a[i]} browser=${b[i]}`;
  if (a.length !== b.length) return `len native=${a.length} browser=${b.length}`;
  return null;
}

function viewBoxSize(svg) {
  const m = svg.match(/viewBox="([\d.eE+\-\s]+)"/);
  if (!m) return null;
  const p = m[1].trim().split(/\s+/).map(Number);
  return p.length === 4 && p[2] > 0 && p[3] > 0 ? { w: p[2], h: p[3] } : null;
}

// Render an SVG string to a PNG of exactly w×h by forcing an intrinsic size and matching
// the Edge window, so native and browser frames align pixel-for-pixel.
function render(svg, name, w, h) {
  const sized = svg.replace(/<svg\b/, `<svg width="${w}" height="${h}"`);
  const svgFile = join(tmp, `${name}.svg`);
  const pngFile = join(tmp, `${name}.png`);
  writeFileSync(svgFile, sized);
  edge([`--window-size=${w},${h}`, "--virtual-time-budget=3000", `--screenshot=${pngFile}`, `file://${svgFile}`]);
  return PNG.sync.read(readFileSync(pngFile));
}

function pixelDelta(sample, nSvg, bSvg) {
  const vb = viewBoxSize(bSvg) || viewBoxSize(nSvg);
  if (!vb) return null;
  const scale = RENDER_MAX / Math.max(vb.w, vb.h);
  const w = Math.max(1, Math.round(vb.w * scale));
  const h = Math.max(1, Math.round(vb.h * scale));
  const nat = render(nSvg, `${sample}-native`, w, h);
  const bro = render(bSvg, `${sample}-browser`, w, h);
  const diff = new PNG({ width: w, height: h });
  const bad = pixelmatch(nat.data, bro.data, diff.data, w, h, { threshold: 0.1 });
  writeFileSync(join(tmp, `${sample}-diff.png`), PNG.sync.write(diff));
  return (bad / (w * h)) * 100;
}

// ---- run --------------------------------------------------------------------
console.log(`comparing ${samples.length} samples (native CLI vs browser)\n`);
console.log("sample".padEnd(22), "structural".padEnd(28), "pixel Δ");
console.log("-".repeat(64));

let structFails = 0;
let pixelFails = 0;
for (const s of samples) {
  let struct = "";
  let pixel = "";
  try {
    const nSvg = nativeSvg(s);
    let bSvg = browserSvg(s);
    let sd = structDiff(structure(nSvg), structure(bSvg));
    if (sd) {
      // A --dump-dom race (the browser's async font fetch) can truncate the DOM;
      // re-extract once. A real divergence persists across the retry.
      const retry = browserSvg(s);
      const sd2 = structDiff(structure(nSvg), structure(retry));
      if (!sd2) {
        bSvg = retry;
        sd = null;
      } else sd = sd2;
    }
    if (sd) {
      struct = `DIFF ${sd}`;
      structFails++;
    } else {
      struct = "match";
    }
    const d = pixelDelta(s, nSvg, bSvg);
    if (d == null) pixel = "n/a";
    else {
      const flag = d > PIXEL_FAIL ? " ✗" : d > PIXEL_WARN ? " ~" : " ✓";
      pixel = `${d.toFixed(3)}%${flag}`;
      if (d > PIXEL_FAIL) pixelFails++;
    }
  } catch (e) {
    struct = `ERROR ${String(e.message || e).slice(0, 40)}`;
    structFails++;
  }
  console.log(s.padEnd(22), struct.padEnd(28), pixel);
}

console.log("-".repeat(64));
console.log(`structural mismatches: ${structFails}   pixel >${PIXEL_FAIL}%: ${pixelFails}`);
console.log(`diffs + renders in ${tmp}`);
process.exit(structFails > 0 || pixelFails > 0 ? 1 : 0);
