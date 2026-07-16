//! @visioncortex/xsvg-compile — compile xsvg to plain SVG in Node.js, no browser.
//!
//! xsvg is an XML graphics format that compiles to plain SVG (the text layout, warps, and
//! gradient meshes SVG never got). This package runs the same WASM compiler the browser
//! uses, backing its platform seams with native Node modules — opentype.js for font
//! metrics + glyph outlining, and a JS shape rasterizer for `in="#shape"` text flow. No
//! native addon, no cross-compilation: one portable wasm + pure-JS deps.
//!
//!   import { compile } from "@visioncortex/xsvg-compile";
//!   const svg = compile(source, { fontDir: "./fonts" }); // synchronous
import { createRequire } from "node:module";
import { EMPTY_FONTS, loadFontDir, type Font, type FontDb } from "./fonts.js";
import { rasterize } from "./rasterize.js";

type Cb = (...args: unknown[]) => unknown;
interface WasmModule {
  compile(
    input: string,
    quality: string,
    sourcemap: boolean,
    measure: Cb,
    metrics: Cb,
    rasterize: Cb,
    outlineRun: Cb,
    advanceWidth: Cb,
  ): string;
}

// wasm-pack --target nodejs emits a CommonJS module that loads the .wasm synchronously.
// createRequire loads it from ESM without any interop guessing. `../pkg` resolves the same
// from src/index.ts and the built dist/index.js (both one level under the package root).
const require = createRequire(import.meta.url);
const wasm = require("../pkg/xsvg_wasm.js") as WasmModule;

// `-x-google-<Name>` families name a Google font in the browser; Node resolves fonts from
// the fontDir instead, so strip the marker (mirrors the browser) for clean output + lookup.
const GOOGLE_PREFIX = "-x-google-";
function stripGooglePrefix(source: string): string {
  return source.replace(
    /font-family\s*=\s*"([^"]*)"/g,
    (_m, v: string) => `font-family="${v.split(GOOGLE_PREFIX).join("")}"`,
  );
}

// The core hands the resolved family verbatim (maybe a comma list); key lookup on the first.
function firstFamily(family: string): string {
  return family.split(",")[0].trim().replace(/^['"]|['"]$/g, "");
}

// "{style} {weight} {size}px {family}" (TextStyle::font_css).
function parseFontCss(css: string): { style: string; size: number; family: string } {
  const m = css.match(/^(\S+)\s+\S+\s+([\d.]+)px\s+(.*)$/);
  return m
    ? { style: m[1], size: parseFloat(m[2]), family: m[3] }
    : { style: "normal", size: 16, family: "" };
}

function measure(fonts: FontDb, text: string, fontCss: string): number {
  const { size, family, style } = parseFontCss(fontCss);
  const font = fonts.resolve(firstFamily(family), style);
  if (!font) return 0;
  try {
    return font.getAdvanceWidth(text, size);
  } catch {
    return 0;
  }
}

function metrics(fonts: FontDb, fontCss: string): number[] {
  const { size, family, style } = parseFontCss(fontCss);
  const font = fonts.resolve(firstFamily(family), style);
  if (!font) return [];
  const s = size / font.unitsPerEm;
  const cap = font.tables.os2?.sCapHeight;
  const ex = font.tables.os2?.sxHeight;
  // [ascent, descent(+), capHeight, xHeight]; 0/absent → the Rust side uses its defaults
  return [font.ascender * s, -font.descender * s, cap ? cap * s : 0, ex ? ex * s : 0];
}

function outlineRun(font: Font | undefined, text: string, size: number, x: number, baseline: number): string {
  if (!font) return "";
  let d = font.getPath(text, x, baseline, size).toPathData(2);
  // opentype.js v2 emits no explicit close commands; close each subpath so a stroked
  // (keyline) glyph has no open gap — the exact fix the browser adapter applies.
  if (d && !d.includes("Z")) d = d.replace(/M/g, (_m, i) => (i === 0 ? "M" : "ZM")) + "Z";
  return d;
}

export interface CompileOptions {
  /** Quality profile: "fast" | "balanced" (default) | "highest". */
  quality?: string;
  /** Emit `data-xsvg-pos="START-END"` byte-range attributes on top-level elements. */
  sourcemap?: boolean;
  /** Directory of `.ttf`/`.otf` fonts for text measurement + `outline="true"` baking.
   *  Without it, text uses default metrics and stays live `<text>` (no outline baking). */
  fontDir?: string;
}

const fontCache = new Map<string, FontDb>();

/** Compile an xsvg source string to a plain-SVG string. Synchronous. */
export function compile(source: string, opts: CompileOptions = {}): string {
  const { quality = "balanced", sourcemap = false, fontDir } = opts;
  let fonts = EMPTY_FONTS;
  if (fontDir) {
    let db = fontCache.get(fontDir);
    if (!db) fontCache.set(fontDir, (db = loadFontDir(fontDir)));
    fonts = db;
  }
  const src = stripGooglePrefix(source);
  return wasm.compile(
    src,
    quality,
    sourcemap,
    (text, css) => measure(fonts, text as string, css as string),
    (css) => metrics(fonts, css as string),
    (d, rowH) => rasterize(d as string, rowH as number),
    (text, fam, _w, style, size, x, b) =>
      outlineRun(fonts.resolve(firstFamily(fam as string), style as string), text as string, size as number, x as number, b as number),
    (text, fam, _w, style, size) => {
      const font = fonts.resolve(firstFamily(fam as string), style as string);
      return font ? font.getAdvanceWidth(text as string, size as number) : NaN;
    },
  );
}

export { loadFontDir } from "./fonts.js";
export type { FontDb } from "./fonts.js";
