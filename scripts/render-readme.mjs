#!/usr/bin/env node
// Render README feature images: CLI-compile each sample to plain SVG, then rasterize it
// with headless Edge (system fonts for live text; outlined text is already baked). No dev
// server. Writes PNGs into assets/readme/. Usage: node scripts/render-readme.mjs
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const EDGE = "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
const CLI = "./target/release/xsvg";
const SCALE = 1.5; // retina-ish crispness
const tmp = mkdtempSync(join(tmpdir(), "xsvg-readme-"));
mkdirSync("assets/readme", { recursive: true });

// [sample, output basename]
const SHOTS = [
  ["mesh", "mesh"],
  ["region-flow", "text-flow"],
  ["justify", "text-justify"],
  ["textpath-effects", "textpath"],
  ["warp-presets", "warp"],
  ["connectors", "connectors"],
  ["boolean", "boolean"],
  ["offset", "offset"],
  ["paragraphs", "paragraphs"],
  ["table", "table"],
  ["plot", "plot"],
  ["pie", "pie"],
  ["lists", "lists"],
  ["theme", "theme"],
];

for (const [sample, out] of SHOTS) {
  const svg = execFileSync(CLI, ["--font-directory", "assets/fonts", `dataset/${sample}.xsvg`], {
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
  const m = svg.match(/viewBox="([\d.\-\s]+)"/);
  if (!m) {
    console.log(`skip ${sample}: no viewBox`);
    continue;
  }
  const [, , vw, vh] = m[1].trim().split(/\s+/).map(Number);
  const w = Math.round(vw * SCALE);
  const h = Math.round(vh * SCALE);
  // force intrinsic size so Edge rasterizes at exactly w×h (no letterbox)
  const sized = svg.replace(/<svg\b/, `<svg width="${w}" height="${h}"`);
  const svgFile = join(tmp, `${out}.svg`);
  writeFileSync(svgFile, sized);
  execFileSync(EDGE, [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--force-device-scale-factor=1",
    `--window-size=${w},${h}`,
    "--virtual-time-budget=4000",
    `--screenshot=assets/readme/${out}.png`,
    `file://${svgFile}`,
  ], { stdio: ["ignore", "ignore", "ignore"] });
  console.log(`wrote assets/readme/${out}.png (${w}x${h})`);
}
