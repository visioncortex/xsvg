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
import { readFileSync } from "node:fs";
import { dirname, resolve as resolvePath } from "node:path";
import { EMPTY_FONTS, loadFontDir, type Font, type FontDb } from "./fonts.js";
import { rasterize } from "./rasterize.js";
import { noPlugins, PluginRegistry, type ImportDecl } from "./plugins.js";

type Cb = (...args: unknown[]) => unknown;
interface WasmModule {
  compile_ext(
    input: string,
    quality: string,
    sourcemap: boolean,
    measure: Cb,
    metrics: Cb,
    rasterize: Cb,
    outlineRun: Cb,
    advanceWidth: Cb,
    resolve: Cb,
    base: string,
    pluginElement: Cb,
    pluginGenerate: Cb,
  ): string;
  compile_fragment_ext(
    input: string,
    quality: string,
    sourcemap: boolean,
    offset: number,
    measure: Cb,
    metrics: Cb,
    rasterize: Cb,
    outlineRun: Cb,
    advanceWidth: Cb,
    resolve: Cb,
    base: string,
    pluginElement: Cb,
    pluginGenerate: Cb,
  ): string;
  fragment_range(input: string, offset: number): Uint32Array;
  dependents(input: string, offset: number): Uint32Array;
  /** The `<x:import>` bindings a document declares, as JSON. */
  imports(input: string): string;
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
  /** Path of the source file — the anchor for resolving relative cross-file
   *  `<use href="…">` links (read synchronously from disk). Defaults to a file in the
   *  current working directory. */
  basePath?: string;
  /** Set false to load no `<x:import>` generator modules, so every plugin element
   *  degrades with a marker (docs/Extensibility.md). Modules are read from disk
   *  relative to `basePath`; a document that imports nothing pays nothing either way. */
  plugins?: boolean;
}

const fontCache = new Map<string, FontDb>();

// Everything a compile call needs beyond the source: the resolved font database, the
// disk resolver, the document's generator modules, and the arguments derived from the
// options. Built once per call and shared by the whole-document and single-fragment
// entry points, so the two can never be lowering against different seams.
function session(source: string, opts: CompileOptions) {
  const { quality = "balanced", sourcemap = false, fontDir } = opts;
  let fonts = EMPTY_FONTS;
  if (fontDir) {
    let db = fontCache.get(fontDir);
    if (!db) fontCache.set(fontDir, (db = loadFontDir(fontDir)));
    fonts = db;
  }
  const base = opts.basePath ?? resolvePath("source.xsvg"); // anchor for relative <use href>
  // Disk-backed resolver: read the dependency relative to the referrer, strip google
  // markers, key by absolute path (stable for the compiler's cycle guard). No pre-walk —
  // the sync compiler calls this on demand as it recurses.
  const resolve = (b: unknown, href: unknown): [string, string] | null => {
    try {
      const abs = resolvePath(dirname(b as string), href as string);
      return [abs, stripGooglePrefix(readFileSync(abs, "utf8"))];
    } catch {
      return null;
    }
  };
  const src = stripGooglePrefix(source);
  // <x:import> is a declaration, so the compiler reports a document's bindings from
  // a parse alone — no throwaway compile round to discover them.
  let plugins = noPlugins;
  if (opts.plugins !== false) {
    let decls: ImportDecl[] = [];
    try {
      decls = JSON.parse(wasm.imports(src));
    } catch {
      decls = [];
    }
    if (decls.length) plugins = PluginRegistry.load(decls, base);
  }
  // The seam callbacks, in the order every compiler entry point takes them.
  const seams = [
    (text: unknown, css: unknown) => measure(fonts, text as string, css as string),
    (css: unknown) => metrics(fonts, css as string),
    (d: unknown, rowH: unknown) => rasterize(d as string, rowH as number),
    (text: unknown, fam: unknown, _w: unknown, style: unknown, size: unknown, x: unknown, b: unknown) =>
      outlineRun(fonts.resolve(firstFamily(fam as string), style as string), text as string, size as number, x as number, b as number),
    (text: unknown, fam: unknown, _w: unknown, style: unknown, size: unknown) => {
      const font = fonts.resolve(firstFamily(fam as string), style as string);
      return font ? font.getAdvanceWidth(text as string, size as number) : NaN;
    },
    resolve,
  ] as const;
  const hosts = [
    (ns: unknown, tag: unknown) => plugins.element(ns as string, tag as string),
    (ns: unknown, tag: unknown, req: unknown) => plugins.generate(ns as string, tag as string, req as string),
  ] as const;
  return { src, quality, sourcemap, base, plugins, seams, hosts };
}

/** Compile an xsvg source string to a plain-SVG string. Synchronous — cross-file
 *  `<use href="…">` links are read from disk on demand, relative to `basePath`. */
export function compile(source: string, opts: CompileOptions = {}): string {
  const s = session(source, opts);
  s.plugins.beginCompile();
  return wasm.compile_ext(s.src, s.quality, s.sourcemap, ...s.seams, s.base, ...s.hosts);
}

/**
 * Re-emit just the top-level element containing byte `offset` (docs/Incremental.md).
 *
 * The markup is byte-identical to that element's span in a full `compile` of the same
 * source, which is what lets a live-editing host swap it into an already-rendered
 * document instead of replacing the whole drawing. Returns null when `offset` is not
 * inside a top-level element (inter-element whitespace, or the root tag itself) —
 * the caller falls back to a full compile.
 *
 * Offsets are UTF-8 byte offsets, matching the `data-xsvg-pos` attributes `sourcemap`
 * emits. Note that `-x-google-` font markers are stripped before compiling, so a
 * document using them must be measured in the same stripped coordinates.
 */
export function compileFragment(source: string, offset: number, opts: CompileOptions = {}): string | null {
  const s = session(source, opts);
  s.plugins.beginCompile();
  try {
    return wasm.compile_fragment_ext(s.src, s.quality, s.sourcemap, offset, ...s.seams, s.base, ...s.hosts);
  } catch {
    return null;
  }
}

/** Byte range of the fragment unit containing `offset`, or null when the offset falls
 *  outside every top-level element. Parse-only — nothing is lowered. */
export function fragmentRange(source: string, offset: number): [number, number] | null {
  const r = wasm.fragment_range(stripGooglePrefix(source), offset);
  return r.length === 2 ? [r[0], r[1]] : null;
}

/** Byte ranges of the other top-level elements that must be re-emitted alongside the
 *  fragment at `offset` — the transitive closure over baked `in="#id"` references.
 *  Parse-only. */
export function dependents(source: string, offset: number): [number, number][] {
  const flat = wasm.dependents(stripGooglePrefix(source), offset);
  const out: [number, number][] = [];
  for (let i = 0; i + 1 < flat.length; i += 2) out.push([flat[i], flat[i + 1]]);
  return out;
}

/**
 * The text the compiler actually sees for a given source — `-x-google-<Name>` font
 * markers resolved away.
 *
 * Every byte offset in this module's incremental API, and every `data-xsvg-pos` the
 * compiler emits, indexes *this* string. A host that maps output back to source must
 * normalize first, or a document using those markers is off by ten bytes per marker
 * ahead of the element it is looking at.
 */
export function normalizeSource(source: string): string {
  return stripGooglePrefix(source);
}

export { loadFontDir } from "./fonts.js";
export type { FontDb } from "./fonts.js";
export { PluginRegistry } from "./plugins.js";
export type { ImportDecl } from "./plugins.js";
