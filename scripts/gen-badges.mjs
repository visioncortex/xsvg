#!/usr/bin/env node
// Generate the README status badges as self-hosted SVGs under assets/readme/.
//
// Why not shields.io / badgen? Those are third-party image CDNs that some networks
// block, and when they're unreachable the README shows broken images. These SVGs are
// served from the repo itself (raw.githubusercontent.com, same as every other README
// image), so they render wherever the rest of the README does. Re-run after a version
// bump to refresh: `node scripts/gen-badges.mjs`.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = (p) => JSON.parse(readFileSync(resolve(root, p), "utf8"));

// Approximate Verdana 11px advance widths — enough to size segments without clipping.
const charWidth = (ch) => {
  if (/[A-Z]/.test(ch)) return 8.0;
  if (/[a-z0-9]/.test(ch)) return 6.6;
  if (ch === " ") return 3.5;
  return 4.0; // . - : / etc.
};
const textWidth = (s) => [...s].reduce((w, c) => w + charWidth(c), 0);

// One flat-style two-segment badge (label on #555, value on `color`).
function badge(label, value, color) {
  const pad = 6; // per side
  const lw = Math.round(textWidth(label) + pad * 2);
  const vw = Math.round(textWidth(value) + pad * 2);
  const w = lw + vw;
  const lx = (lw / 2) * 10; // text x in 1/10 units (rendered at 10x, scaled 0.1) for crisp centering
  const vx = (lw + vw / 2) * 10;
  const lt = textWidth(label) * 10;
  const vt = textWidth(value) * 10;
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="20" role="img" aria-label="${esc(label)}: ${esc(value)}">
  <title>${esc(label)}: ${esc(value)}</title>
  <linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#bbb" stop-opacity=".1"/><stop offset="1" stop-opacity=".1"/></linearGradient>
  <clipPath id="r"><rect width="${w}" height="20" rx="3" fill="#fff"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="${lw}" height="20" fill="#555"/>
    <rect x="${lw}" width="${vw}" height="20" fill="${color}"/>
    <rect width="${w}" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="110" transform="scale(.1)" text-rendering="geometricPrecision">
    <text x="${lx}" y="150" fill="#010101" fill-opacity=".3" textLength="${lt}">${esc(label)}</text>
    <text x="${lx}" y="140" textLength="${lt}">${esc(label)}</text>
    <text x="${vx}" y="150" fill="#010101" fill-opacity=".3" textLength="${vt}">${esc(value)}</text>
    <text x="${vx}" y="140" textLength="${vt}">${esc(value)}</text>
  </g>
</svg>
`;
}

const viewer = pkg("packages/xsvg-viewer/package.json");
const compile = pkg("packages/xsvg-compile/package.json");

const NPM_RED = "#cb3837";
const BLUE = "#007ec6";

const out = [
  ["assets/readme/badge-viewer.svg", badge("xsvg-viewer", `v${viewer.version}`, NPM_RED)],
  ["assets/readme/badge-compile.svg", badge("xsvg-compile", `v${compile.version}`, NPM_RED)],
  ["assets/readme/badge-license.svg", badge("license", viewer.license || "MIT", BLUE)],
];

for (const [path, svg] of out) {
  writeFileSync(resolve(root, path), svg);
  console.log("wrote", path);
}
