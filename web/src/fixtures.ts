// Font-metric fixture generator (dev tool, not shipped in the production build).
//
// Measures per-glyph advance widths and vertical metrics for a few common system
// fonts via canvas measureText, at a 100px base size. Emits one JSON file per
// font for crates/xsvg-core/tests/fixtures/, consumed by the integration test's
// FixtureMeasurer. Open at http://localhost:5173/fixtures.html.

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
