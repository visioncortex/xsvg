// Font-metric + region fixture generator (dev tool, not shipped in production).
//
// Measures per-glyph advance widths and vertical metrics for a few common system
// fonts via canvas measureText, at a 100px base size. Also rasterizes a few shapes
// (via the browser Shaper path) into coarse span tables. Emits JSON for
// crates/xsvg-core/tests/fixtures/, consumed by the integration tests. Open at
// http://localhost:5173/fixtures/.

import { rasterize } from "../core/compiler";

const FONTS = ["Arial", "Times New Roman", "Courier New", "Georgia"];
const BASE = 100;

interface FontFixture {
  family: string;
  baseSize: number;
  ascent: number;
  descent: number;
  capHeight: number;
  xHeight: number;
  space: number;
  chars: Record<string, number>;
}

const round = (n: number) => Math.round(n * 1000) / 1000;

function measureFont(family: string): FontFixture {
  const ctx = document.createElement("canvas").getContext("2d")!;
  ctx.font = `${BASE}px '${family}'`;
  const chars: Record<string, number> = {};
  for (let i = 32; i < 127; i++) {
    const ch = String.fromCharCode(i);
    chars[ch] = round(ctx.measureText(ch).width);
  }
  const box = ctx.measureText("Hg"); // em box is font-level
  return {
    family,
    baseSize: BASE,
    ascent: round(box.fontBoundingBoxAscent),
    descent: round(box.fontBoundingBoxDescent),
    capHeight: round(ctx.measureText("H").actualBoundingBoxAscent),
    xHeight: round(ctx.measureText("x").actualBoundingBoxAscent),
    space: chars[" "],
    chars,
  };
}

const slug = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, "-");

const fonts: Record<string, FontFixture> = {};
for (const f of FONTS) fonts[f] = measureFont(f);

const combined = JSON.stringify({ baseSize: BASE, fonts }, null, 2);

// headless-capture marker (base64; content is ASCII so btoa is safe)
document.getElementById("out")!.textContent = "FIXTURE_B64:" + btoa(combined) + ":END";

// human-readable view + per-font downloads
document.getElementById("readable")!.textContent = combined;
const downloads = document.getElementById("downloads")!;
for (const [name, fix] of Object.entries(fonts)) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([JSON.stringify(fix, null, 2)], { type: "application/json" }));
  a.download = `${slug(name)}.json`;
  a.textContent = `download ${slug(name)}.json`;
  downloads.appendChild(a);
}

// ---- region fixtures: coarse shape rasters (browser Shaper) -----------------

interface RegionFixture {
  name: string;
  minX: number;
  minY: number;
  w: number;
  h: number;
  rowH: number;
  rows: ([number, number] | null)[]; // inside [left,right] per row, null if empty
}

const SHAPES: { name: string; d: string; rowH: number }[] = [
  { name: "triangle-down", d: "M10,10 L110,10 L60,130 Z", rowH: 8 },
  { name: "circle", d: "M0,60 a60,60 0 1,0 120,0 a60,60 0 1,0 -120,0 Z", rowH: 8 },
  { name: "diamond", d: "M60,10 L120,70 L60,130 L0,70 Z", rowH: 8 },
];

function shapeFixture(name: string, d: string, rowH: number): RegionFixture {
  const a = rasterize(d, rowH);
  const rows: ([number, number] | null)[] = [];
  for (let i = 5; i + 1 < a.length; i += 2) {
    const l = a[i];
    const r = a[i + 1];
    rows.push(Number.isNaN(l) || Number.isNaN(r) ? null : [round(l), round(r)]);
  }
  return {
    name,
    minX: round(a[0]),
    minY: round(a[1]),
    w: round(a[2]),
    h: round(a[3]),
    rowH: round(a[4]),
    rows,
  };
}

const regions = SHAPES.map((s) => shapeFixture(s.name, s.d, s.rowH));
const regionJson = JSON.stringify({ regions }, null, 2);
document.getElementById("regionsOut")!.textContent = "REGION_B64:" + btoa(regionJson) + ":END";
document.getElementById("regionsReadable")!.textContent = regionJson;
