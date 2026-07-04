// Source-map helpers for the interactive viewer's element→source projection.
//
// The WASM compiler tags emitted elements with `data-xsvg-pos="START-END"` — byte
// offsets into the xsvg source (UTF-8). CodeMirror positions are UTF-16 string
// indices, so `ByteIndex` converts between the two. Synthesized subtrees tag only
// their root element, so `nearestSource` walks up to the nearest tagged ancestor.

export interface SourceRange {
  /** UTF-8 byte offset into the xsvg source. */
  start: number;
  end: number;
}

/**
 * Byte-offset → UTF-16-index converter for one source string. Built once
 * (O(n) over code points); lookups are O(log n) via binary search over code-point
 * boundaries. Element ranges begin/end on ASCII delimiters, so they always land on
 * a boundary — but this stays correct for any UTF-8 offset.
 */
export class ByteIndex {
  private bytes: number[] = [0]; // byte offset of each code-point boundary
  private units: number[] = [0]; // matching UTF-16 index

  constructor(source: string) {
    let b = 0;
    let u = 0;
    for (const ch of source) {
      b += utf8Len(ch.codePointAt(0)!);
      u += ch.length; // 1 for BMP, 2 for a surrogate pair
      this.bytes.push(b);
      this.units.push(u);
    }
  }

  /** UTF-16 string index at (or just before) the given byte offset. */
  toStr(byte: number): number {
    let lo = 0;
    let hi = this.bytes.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.bytes[mid] <= byte) lo = mid;
      else hi = mid - 1;
    }
    return this.units[lo];
  }
}

function utf8Len(cp: number): number {
  if (cp < 0x80) return 1;
  if (cp < 0x800) return 2;
  if (cp < 0x10000) return 3;
  return 4;
}

/** Parse `data-xsvg-pos="a-b"` on an element, or null if absent/malformed. */
export function posAttr(el: Element): SourceRange | null {
  const v = el.getAttribute("data-xsvg-pos");
  if (!v) return null;
  const dash = v.indexOf("-");
  if (dash < 0) return null;
  const start = parseInt(v.slice(0, dash), 10);
  const end = parseInt(v.slice(dash + 1), 10);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  return { start, end };
}

/**
 * Walk up from `el` (inclusive) to the nearest ancestor carrying `data-xsvg-pos`,
 * not going above `root`. Returns the tagged element and its source range, or null.
 */
export function nearestSource(
  el: Element | null,
  root: Element,
): { el: Element; range: SourceRange } | null {
  let cur: Element | null = el;
  while (cur) {
    const range = posAttr(cur);
    if (range) return { el: cur, range };
    if (cur === root) break;
    cur = cur.parentElement;
  }
  return null;
}
