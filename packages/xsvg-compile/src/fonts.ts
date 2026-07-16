// Fonts for the Node compiler, loaded from a directory with opentype.js and classified
// into roles by family name + italic bit — the same scheme as the native `xsvg` CLI.
// opentype.js supplies advance widths and glyph outlines; a caller supplies the directory
// (`fontDir`), so the package bundles no fonts and carries no font-license baggage.
import { readdirSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
// opentype.js ships no types; we narrow to just what we use. It's CJS — Node's ESM loader
// exposes it only as the default export (a namespace import misses `parse`).
import opentype from "opentype.js";

export interface Font {
  unitsPerEm: number;
  ascender: number;
  descender: number;
  tables: { os2?: { sCapHeight?: number; sxHeight?: number; fsSelection?: number }; head?: { macStyle?: number } };
  names: { fontFamily?: Record<string, string>; preferredFamily?: Record<string, string> };
  getAdvanceWidth(text: string, fontSize: number): number;
  getPath(text: string, x: number, y: number, fontSize: number): { toPathData(digits?: number): string };
}

const parse = (opentype as { parse(buf: ArrayBuffer): Font }).parse;

/** The loaded fonts, one slot per role. */
export interface FontDb {
  resolve(family: string, style: string): Font | undefined;
}

function firstName(rec: Record<string, string> | undefined): string {
  if (!rec) return "";
  return rec.en ?? Object.values(rec)[0] ?? "";
}

function familyOf(f: Font): string {
  return (firstName(f.names.preferredFamily) || firstName(f.names.fontFamily)).toLowerCase();
}

function isItalic(f: Font): boolean {
  const fs = f.tables.os2?.fsSelection;
  if (typeof fs === "number") return (fs & 0x01) !== 0; // OS/2 ITALIC bit
  const mac = f.tables.head?.macStyle;
  return typeof mac === "number" && (mac & 0x02) !== 0;
}

/** Load every `.ttf`/`.otf` in `dir`, routing each into a role by family name + italic
 *  bit (the first file to claim a role wins). Unreadable/unparseable files are skipped. */
export function loadFontDir(dir: string): FontDb {
  let display: Font | undefined;
  let sans: Font | undefined;
  let sansItalic: Font | undefined;
  for (const name of readdirSync(dir)) {
    const ext = extname(name).toLowerCase();
    if (ext !== ".ttf" && ext !== ".otf") continue;
    let font: Font;
    try {
      const buf = readFileSync(join(dir, name));
      font = parse(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));
    } catch {
      continue;
    }
    const fam = familyOf(font);
    if (fam.includes("anton")) display ??= font;
    else if (isItalic(font)) sansItalic ??= font;
    else sans ??= font;
  }
  return {
    resolve(family: string, style: string): Font | undefined {
      const fam = family.toLowerCase();
      const italic = /italic|oblique/.test(style.toLowerCase());
      if (fam.includes("anton")) return display ?? sans;
      if (italic) return sansItalic ?? sans;
      return sans;
    },
  };
}

/** An empty db (no fonts): text falls back to default metrics and stays live `<text>`. */
export const EMPTY_FONTS: FontDb = { resolve: () => undefined };
