// Checks that a real SVG renderer reconstructs <x:mesh> output *exactly*.
//
// A mesh lowers to tiny texel-aligned PNGs, placed so that bilinear upsampling —
// the renderer's own image filter — reproduces each region's fitted colour field.
// That is a claim about rendering, so this renders: every mesh region in
// dataset/mesh.xsvg goes through resvg, and each pixel inside its clip is compared
// with the bilinear upsample the placement was designed around. It must land within
// 8-bit rounding.
//
// It has caught a real regression: with no `image-rendering`, resvg chooses bicubic,
// which drifts up to ~9/255 off the field — invisible on the page, but no longer the
// field. The output now names bilinear explicitly; drop that and this fails.
//
//   npm run check:mesh        (needs `resvg` on PATH and a release CLI build)

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PNG } from "pngjs";

// rounding alone accounts for 0.5; anything past 1 is a different reconstruction
const TOLERANCE = 1;
// stay this many device pixels inside a clip, so antialiased edges don't count
const INSET = 4;

if (spawnSync("resvg", ["--version"]).status !== 0) {
  console.log("mesh-raster: skipped — resvg not on PATH (cargo install resvg)");
  process.exit(0);
}

const tmp = mkdtempSync(join(tmpdir(), "xsvg-mesh-"));
const svg = execFileSync("./target/release/xsvg", ["dataset/mesh.xsvg"], { encoding: "utf8" });

// The sample overlays a dashed wireframe and corner dots on its Coons patches; they
// are drawn on top of the gradient, so they are not part of what is measured.
const bare = svg
  .replace(/<g[^>]*stroke-dasharray[^>]*>[\s\S]*?<\/g>/g, "")
  .replace(/<circle[^>]*\/>/g, "");
const root = svg.match(/<svg[^>]*>/)[0];

function render(source, name, zoom) {
  const file = join(tmp, `${name}.svg`);
  writeFileSync(file, source);
  execFileSync("resvg", ["--zoom", String(zoom), file, join(tmp, `${name}.png`)]);
  return PNG.sync.read(readFileSync(join(tmp, `${name}.png`)));
}

/** Bilinear sample of a texel grid at texel-space (u, v); centres at i + 0.5, edges clamped. */
function bilinear(tex, u, v, c) {
  const x = Math.min(Math.max(u - 0.5, 0), tex.width - 1);
  const y = Math.min(Math.max(v - 0.5, 0), tex.height - 1);
  const x0 = Math.floor(x), y0 = Math.floor(y);
  const x1 = Math.min(x0 + 1, tex.width - 1), y1 = Math.min(y0 + 1, tex.height - 1);
  const fx = x - x0, fy = y - y0;
  const at = (i, j) => tex.data[(j * tex.width + i) * 4 + c];
  return (
    at(x0, y0) * (1 - fx) * (1 - fy) + at(x1, y0) * fx * (1 - fy) +
    at(x0, y1) * (1 - fx) * fy + at(x1, y1) * fx * fy
  );
}

const images = [
  ...svg.matchAll(
    /<image x="([-\d.]+)" y="([-\d.]+)" width="([\d.]+)" height="([\d.]+)"[^>]*?clip-path="url\(#([^)]+)\)" href="data:image\/png;base64,([^"]+)"/g,
  ),
];
if (images.length === 0) throw new Error("no mesh images in the compiled sample — did the lowering change shape?");

let failed = 0;
for (const zoom of [1, 3]) {
  const page = render(bare, `page-${zoom}`, zoom);
  for (const m of images) {
    const [x, y, w, h] = m.slice(1, 5).map(Number);
    const cid = m[5];
    const tex = PNG.sync.read(Buffer.from(m[6], "base64"));
    const clip = svg.match(new RegExp(`<clipPath id="${cid}"[^>]*>[\\s\\S]*?</clipPath>`))[0];
    const mask = render(
      `${root}<defs>${clip}</defs><rect x="-1e5" y="-1e5" width="2e5" height="2e5" fill="#000"/>` +
        `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="#fff" clip-path="url(#${cid})"/></svg>`,
      `mask-${cid}-${zoom}`,
      zoom,
    );
    const inside = (px, py) => {
      for (let dy = -INSET; dy <= INSET; dy++)
        for (let dx = -INSET; dx <= INSET; dx++) {
          const qx = px + dx, qy = py + dy;
          if (qx < 0 || qy < 0 || qx >= mask.width || qy >= mask.height) return false;
          if (mask.data[(qy * mask.width + qx) * 4] !== 255) return false;
        }
      return true;
    };
    let worst = 0, n = 0;
    for (let py = 0; py < page.height; py++)
      for (let px = 0; px < page.width; px++) {
        if (mask.data[(py * mask.width + px) * 4] !== 255 || !inside(px, py)) continue;
        const u = (((px + 0.5) / zoom - x) / w) * tex.width;
        const v = (((py + 0.5) / zoom - y) / h) * tex.height;
        for (let c = 0; c < 3; c++) {
          const got = page.data[(py * page.width + px) * 4 + c];
          worst = Math.max(worst, Math.abs(got - bilinear(tex, u, v, c)));
        }
        n++;
      }
    const ok = n > 0 && worst <= TOLERANCE;
    if (!ok) failed++;
    console.log(
      `${ok ? "ok  " : "FAIL"}  ${cid.padEnd(16)} ${zoom}x  ${String(n).padStart(7)} px  max error ${worst.toFixed(2)}/255`,
    );
  }
}

if (failed) {
  console.error(`mesh-raster: ${failed} region(s) not reconstructed within ${TOLERANCE}/255 of the fitted field`);
  process.exit(1);
}
console.log(`mesh-raster: every region exact to ${TOLERANCE}/255 at 1x and 3x`);
